import copy
import hashlib
import json
import subprocess
from pathlib import Path
import unittest
from unittest.mock import Mock, patch
from types import SimpleNamespace

import receipts
import run


class Guards(unittest.TestCase):
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

    def test_structured_arguments_are_not_shell_code(self):
        self.assertEqual(run.arguments({'market': {'hub_id': 1}, 'amount': '$(no)'}),
                         ['--market', '{"hub_id":1}', '--amount', '$(no)'])

    def test_committed_values_preserve_precision_and_reject_duplicates(self):
        self.assertEqual(receipts.plain({'i128': str(2**100)}), str(2**100))
        self.assertEqual(receipts.plain({'map': [{'key': {'symbol': 'amount'}, 'val': {'i128': '20'}}]}), {'amount': '20'})
        with self.assertRaises(ValueError):
            receipts.plain({'map': [{'key': {'symbol': 'a'}, 'val': 'void'}] * 2})


class RecoveryKeys(unittest.TestCase):
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


if __name__ == '__main__':
    unittest.main()
