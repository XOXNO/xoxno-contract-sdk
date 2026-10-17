import copy
import hashlib
import json
import re
import subprocess
from pathlib import Path
import unittest
from unittest.mock import Mock, patch
from types import SimpleNamespace

import receipts
import run
import advanced


class Guards(unittest.TestCase):
    def test_fee_headroom_changes_only_unsigned_fee_fields(self):
        transaction = {'fee': 1100, 'ext': {'v1': {'resource_fee':'1000'}}, 'operations':[{'host':'unchanged'}]}
        envelope = {'tx': {'tx':transaction}}
        run.add_fee_headroom(envelope)
        self.assertEqual(transaction, {'fee':101100,'ext':{'v1':{'resource_fee':'101000'}}, 'operations':[{'host':'unchanged'}]})
        transaction['fee'] = 2**32-1
        with self.assertRaises(ValueError):
            run.add_fee_headroom(envelope)

    def test_testnet_required_on_both_sides(self):
        config = {'network_passphrase': run.PASSPHRASE}
        run.validate_network(config, {'passphrase': run.PASSPHRASE})
        for local, remote in [('Public Global Stellar Network ; September 2015', run.PASSPHRASE),
                              (run.PASSPHRASE, 'Public Global Stellar Network ; September 2015')]:
            with self.assertRaises(ValueError):
                run.validate_network({'network_passphrase': local}, {'passphrase': remote})

    def test_no_empty_partial_or_error_report_can_pass(self):
        report = {'cases': dict.fromkeys(run.REQUIRED, True), 'errors': [],
                  'open_accounts': [], 'unknown_outcome': False}
        self.assertEqual(run.final_status(report), 'PASS')
        for changes in ({'cases': {}}, {'cases': {'cleanup': True}}, {'errors': ['failure']},
                        {'open_accounts': [{'account_id': 1}]}, {'unknown_outcome': True}, {'residual_balances': {'vault': 1}},
                        {'cases': dict.fromkeys(run.REQUIRED, False)}):
            self.assertEqual(run.final_status({**report, **changes}), 'FAIL')
        self.assertEqual(run.final_status({**report, 'blend_open': True}), 'FAIL')

    def test_structured_arguments_are_not_shell_code(self):
        self.assertEqual(run.arguments({'market': {'hub_id': 1}, 'amount': '$(no)'}),
                         ['--market', '{"hub_id":1}', '--amount', '$(no)'])

    def test_committed_values_preserve_precision_and_reject_duplicates(self):
        self.assertEqual(receipts.plain({'i128': str(2**100)}), str(2**100))
        self.assertEqual(receipts.plain({'map': [{'key': {'symbol': 'amount'}, 'val': {'i128': '20'}}]}), {'amount': '20'})
        with self.assertRaises(ValueError):
            receipts.plain({'map': [{'key': {'symbol': 'a'}, 'val': 'void'}] * 2})
        self.assertEqual(receipts.plain({'map': [{'key': {'u32': 2}, 'val': {'i128': '10'}}]}), {'2': '10'})

    def test_quote_identity_and_freshness(self):
        quote = dict(from_='unused', to='B', amountIn='12', amountOutMin='1', routeXdr='eA==', snapshot={'ledger': 100})
        quote['from'] = 'A'
        self.assertEqual(advanced.validate_quote(quote, 'A', 'B', 12, 101), '78')
        for changes in ({'from': 'C'}, {'amountIn':'13'}, {'amountOutMin':'0'}, {'routeXdr':'!'},
                        {'snapshot': {'ledger': 80}}, {'snapshot': {'ledger': 102}}):
            with self.assertRaises((ValueError, KeyError)):
                advanced.validate_quote({**quote, **changes}, 'A', 'B', 12, 101)

    def test_invalid_route_preserves_outer_payload_and_changes_only_version(self):
        payload = {'map': [{'key': {'symbol':'ops'}, 'val': {'bytes':'010001'}},
                           {'key': {'symbol':'assets'}, 'val': {'vec':[]}}]}
        with patch.object(advanced, 'decode', return_value=payload), patch.object(advanced, 'codec', return_value='eA==') as encode:
            self.assertEqual(advanced.invalid_route_version('00'), '78')
        self.assertEqual(json.loads(encode.call_args.args[2]), {'map': [
            {'key': {'symbol':'ops'}, 'val': {'bytes':'ff0001'}},
            {'key': {'symbol':'assets'}, 'val': {'vec':[]}}]})

    def test_advanced_gate_cannot_pass_on_core_cases(self):
        report = {'cases': dict.fromkeys(run.REQUIRED, True), 'errors': [], 'open_accounts': [], 'unknown_outcome': False}
        self.assertEqual(run.final_status(report, advanced.AdvancedRun.required), 'FAIL')

    def test_coverage_inventory_names_every_facade_write(self):
        source = (run.ROOT / 'crates/xoxno-contract-sdk/src/lending/facade.rs').read_text()
        writes = source[source.index('pub fn open_account'):source.index('pub fn account_exists')]
        inventory = Path(__file__).with_name('COVERAGE.md').read_text()
        for name in set(re.findall(r'pub fn (\w+)', writes)) - {'resolve_account'}:
            self.assertIn('`' + name + '`', inventory)

    def test_receipt_account_id_becomes_a_json_number_before_nested_requests(self):
        fake = SimpleNamespace(invoke=Mock(return_value='69'), report={'open_accounts': []},
                               save=Mock(), view=Mock(return_value='adapter'), advanced_account='adapter', nft='nft')
        account = advanced.AdvancedRun.open(fake, account_id=0, spoke_id=3, assets=[])
        encoded = run.arguments({'request': {'account_id': account}})[1]
        self.assertIs(type(json.loads(encoded)['account_id']), int)
        self.assertEqual(account, 69)


class RecoveryKeys(unittest.TestCase):
    def test_friendbot_timeout_uses_chain_balance_before_another_funding_attempt(self):
        import tempfile
        with tempfile.TemporaryDirectory() as directory:
            fake = SimpleNamespace(owner='G', output=Path(directory), net=[], config={'asset':'C'}, report={},
                                   cli=Mock(return_value=subprocess.CompletedProcess([],0,stdout='"100000000000"',stderr='')))
            with patch.object(run.subprocess, 'run', return_value=subprocess.CompletedProcess([],28,stdout='',stderr='timeout')) as fund:
                run.Run.fund_owner(fake)
                self.assertEqual(fund.call_count,1)
                self.assertTrue(fake.report['friendbot_funding_confirmed_by_balance'])

    def test_cleanup_failure_keeps_keys_even_without_open_accounts(self):
        report = {'cases': {}, 'errors': [], 'open_accounts': [], 'unknown_outcome': False}
        fake = SimpleNamespace(report=report, setup=Mock(), save=Mock(),
                               scenarios=Mock(side_effect=ValueError('sweep failed')),
                               cleanup=Mock(side_effect=ValueError('RPC unavailable')))
        with patch.object(run, 'Run', return_value=fake), \
                patch.object(run.tempfile, 'mkdtemp', return_value='/private/recovery'), \
                patch.object(run.shutil, 'rmtree') as delete, \
                patch.object(run.sys, 'argv', ['run.py']), patch('builtins.print'):
            self.assertEqual(run.main(), 1)
            delete.assert_not_called()
        self.assertEqual(report['private_state_dir'], '/private/recovery')


class ReceiptBinding(unittest.TestCase):
    def setUp(self):
        self.host = {'invoke_contract': {'contract_address': 'C', 'function_name': 'deposit', 'args': [{'i128': '12'}]}}
        self.envelope = {'tx': {'tx': {'source_account': 'G', 'operations': [
            {'body': {'invoke_host_function': {'host_function': self.host}}}]}}}
        self.digest = hashlib.sha256(b'x').hexdigest()
        self.receipt = {'jsonrpc': '2.0', 'id': 1, 'result': {'status': 'SUCCESS', 'txHash': self.digest,
                       'ledger': 12, 'envelopeXdr': 'env', 'resultXdr': 'result', 'resultMetaXdr': 'meta'}}
        self.values = {'env': self.envelope,
                       'result': {'result': {'tx_success': [{'op_inner': {'invoke_host_function': {'success': self.digest}}}]}},
                       'meta': {'v4': {'soroban_meta': {'return_value': {'u64': '9'}}, 'operations': [{'events': []}]}}}

    def verify(self):
        with patch.object(receipts, 'decode', side_effect=lambda kind, value: self.values[value]), \
                patch.object(receipts, 'codec', return_value='eA=='):
            return receipts.verify(self.receipt, self.digest, run.PASSPHRASE, 'G', self.host)

    def test_valid_receipt(self):
        self.assertEqual(self.verify(), '9')

    def test_reject_status_hash_source_args_and_return_corruption(self):
        for mutation in ('status', 'hash', 'source', 'args', 'return'):
            with self.subTest(mutation=mutation):
                self.setUp()
                if mutation == 'status':
                    self.receipt['result']['status'] = 'FAILED'
                elif mutation == 'hash':
                    self.receipt['result']['txHash'] = '0' * 64
                elif mutation == 'source':
                    self.envelope['tx']['tx']['source_account'] = 'attacker'
                elif mutation == 'args':
                    changed = copy.deepcopy(self.host)
                    changed['invoke_contract']['args'] = []
                    self.host = changed
                else:
                    self.values['result']['result']['tx_success'][0]['op_inner']['invoke_host_function']['success'] = 'bad'
                with self.assertRaises(ValueError):
                    self.verify()

    def test_classic_receipts_bind_operation_and_committed_result(self):
        for operation in ('change_trust', 'create_account'):
            body = {operation: {'destination': 'owner'}}
            self.envelope['tx']['tx']['operations'][0]['body'] = body
            self.values['result'] = {'result': {'tx_success': [{'op_inner': {operation: 'success'}}]}}
            with patch.object(receipts, 'decode', side_effect=lambda kind, value: self.values[value]), \
                    patch.object(receipts, 'codec', return_value='eA=='):
                receipts.verify_classic(self.receipt, self.digest, run.PASSPHRASE, 'G', body)
                with self.assertRaises(ValueError):
                    receipts.verify_classic(self.receipt, self.digest, run.PASSPHRASE, 'G', {operation: {}})
                self.values['result']['result']['tx_success'][0]['op_inner'][operation] = 'malformed'
                with self.assertRaises(ValueError):
                    receipts.verify_classic(self.receipt, self.digest, run.PASSPHRASE, 'G', body)


class Submission(unittest.TestCase):
    def setUp(self):
        import tempfile
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        root = Path(self.temporary.name)
        self.run = run.Run({'rpc_url': 'https://invalid', 'network_passphrase': run.PASSPHRASE},
                           root / 'evidence', root / 'wasm', root / 'keys')
        self.run.payer = 'G'
        self.hash = 'a' * 64
        self.host = {'invoke_contract': {'function_name': 'deposit'}}
        self.envelope = {'tx': {'tx': {'source_account': 'G', 'operations': [{'body': {'invoke_host_function': {'host_function': self.host}}}]}}}
        self.envelope['tx']['tx'].update(fee=1100, ext={'v1':{'resource_fee':'1000'}})
        self.run.cli = lambda args, **kwargs: subprocess.CompletedProcess(args, 0, stdout=(self.hash if args[:2] == ['tx', 'hash'] else 'built'), stderr='')
        self.run.sign_auth = lambda value, expected: value

    def test_reject_unrelated_auth_before_reading_private_key(self):
        self.envelope['tx']['tx']['operations'][0]['body']['invoke_host_function']['auth'] = [
            {'credentials': {'address': {'address': 'owner'}},
             'root_invocation': {'function': {'contract_fn': {'function_name': 'unrelated'}}}}]
        self.run.owner = 'owner'
        with patch.object(run, 'decode', return_value=self.envelope):
            with self.assertRaisesRegex(ValueError, 'authorization root'):
                run.Run.sign_auth(self.run, 'assembled', self.host)

    def command(self, command, **kwargs):
        kwargs['stderr'].write('Signing transaction: ' + self.hash + '\n')
        return subprocess.CompletedProcess(command, 1)

    def execute(self):
        with patch.object(run, 'codec', return_value='built'):
            return self.run.mutate('deposit', ['contract', 'invoke', '--', 'deposit'])

    def test_cli_error_recovers_only_committed_return_without_resubmit(self):
        self.run.rpc = lambda method, params: {'result': {'status': 'SUCCESS'}}
        with patch.object(run, 'decode', return_value=self.envelope), \
                patch.object(run.subprocess, 'run', side_effect=self.command) as submit, \
                patch.object(run, 'verify', return_value='42'):
            self.assertEqual(self.execute(), '42')
            self.assertEqual(submit.call_count, 1)
            self.assertFalse(self.run.report['unknown_outcome'])

    def test_unconfirmed_or_corrupt_receipt_blocks_dependent_mutations(self):
        for status in ('NOT_FOUND', 'SUCCESS'):
            with self.subTest(status=status):
                self.setUp()
                self.run.rpc = lambda method, params: {'result': {'status': status}}
                with patch.object(run, 'decode', return_value=self.envelope), \
                        patch.object(run.subprocess, 'run', side_effect=self.command) as submit, \
                        patch.object(run.time, 'sleep'), \
                        patch.object(run, 'verify', side_effect=ValueError('corrupt')):
                    with self.assertRaises((ValueError, RuntimeError)):
                        self.execute()
                    self.assertTrue(self.run.report['unknown_outcome'])
                    with self.assertRaises(ValueError):
                        self.execute()
                    self.assertEqual(submit.call_count, 1)

    def test_timeout_retries_only_reads(self):
        failure = subprocess.CompletedProcess([], 1, stdout='', stderr='client error (SendRequest)')
        for args, count in ((['contract', 'invoke', '--send=no'], 3), (['keys', 'generate', 'owner'], 1)):
            with patch.object(run.subprocess, 'run', return_value=failure) as command, patch.object(run.time, 'sleep'):
                with self.assertRaises(RuntimeError):
                    run.Run.cli(self.run, args)
                self.assertEqual(command.call_count, count)


if __name__ == '__main__':
    unittest.main()
