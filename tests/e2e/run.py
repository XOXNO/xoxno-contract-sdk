#!/usr/bin/env python3
"""Deploy SDK examples and check their outcomes on the existing Stellar testnet."""
import argparse
import base64
import binascii
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request

from receipts import codec, decode, verify

ROOT = Path(__file__).resolve().parents[2]
PASSPHRASE = 'Test SDF Network ; September 2015'
EXAMPLES = ('account_basics', 'simple_deposit', 'market_picker', 'lending_vault', 'price_reader', 'liquidator')
REQUIRED = {'network', 'deployment', 'prices', 'account_lifecycle', 'reopen',
            'simple_deposit', 'market_picker', 'lending_vault', 'flash_loan',
            'healthy_liquidation', 'sweep', 'authorization', 'cleanup'}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def arguments(values):
    return [part for key, value in values.items() for part in
            ('--' + key, json.dumps(value, separators=(',', ':')) if isinstance(value, (dict, list, bool)) else str(value))]


def validate_network(config, remote):
    require(config['network_passphrase'] == PASSPHRASE and remote.get('passphrase') == PASSPHRASE,
            'refusing a network other than Stellar testnet')


def checked_invocation(envelope, source, expected):
    transaction = envelope['tx']['tx']
    require(transaction['source_account'] == source and len(transaction['operations']) == 1,
            'prepared transaction changed source or operation count')
    invocation = transaction['operations'][0]['body']['invoke_host_function']
    require(invocation['host_function'] == expected, 'prepared transaction changed requested operation')
    return invocation


def final_status(report):
    return ('PASS' if set(report['cases']) == REQUIRED
            and all(report['cases'].values()) and not report['errors']
            and not report['open_accounts'] and not report['unknown_outcome']
            and not any(report.get('residual_balances', {}).values()) else 'FAIL')


class Run:
    def __init__(self, config, output, wasm_dir, private):
        self.config, self.output, self.wasm_dir = config, output, wasm_dir
        output.mkdir(parents=True, exist_ok=False)
        self.private = Path(private)
        self.net = ['--rpc-url', config['rpc_url'], '--network-passphrase', config['network_passphrase']]
        self.report = {'status': 'RUNNING', 'cases': {}, 'errors': [], 'transactions': [],
                       'contracts': {}, 'open_accounts': [], 'unknown_outcome': False, 'config': config}
        self.index = 0
        self.env = {key: value for key, value in os.environ.items()
                    if not key.startswith('STELLAR_') and key != 'RUST_LOG'}
        self.env['NO_COLOR'] = '1'

    def save(self):
        (self.output / 'report.json').write_text(json.dumps(self.report, indent=2) + '\n')

    def case(self, name):
        self.report['cases'][name] = True
        self.save()
        print('PASS', name, flush=True)

    def rpc(self, method, params=None):
        payload = {'jsonrpc': '2.0', 'id': 1, 'method': method, 'params': params or {}}
        request = urllib.request.Request(self.config['rpc_url'], json.dumps(payload).encode(),
                                         {'Content-Type': 'application/json', 'User-Agent': 'xoxno-sdk-e2e'})
        with urllib.request.urlopen(request, timeout=40) as response:
            data = json.load(response)
        require(data.get('id') == 1 and data.get('jsonrpc') == '2.0' and 'error' not in data,
                f'{method}: RPC error: {data}')
        return data

    def cli(self, args, role='owner', stdin=None, check=True):
        command = ['stellar', '--config-dir', str(self.private / role), '--no-cache', *args]
        result = subprocess.run(command, input=stdin, text=True, capture_output=True,
                                timeout=180, env=self.env, cwd=ROOT)
        if check and result.returncode:
            raise RuntimeError(result.stderr[-4000:])
        return result

    def view(self, contract, method, **values):
        result = self.cli(['contract', 'invoke', *self.net, '--id', contract,
                           '--source', self.payer, '--send=no', '--', method, *arguments(values)])
        value = json.loads(result.stdout)
        self.index += 1
        (self.output / f'{self.index:03d}-view-{method}.json').write_text(json.dumps(value, indent=2) + '\n')
        return value

    def instance(self, address):
        result = self.cli(['ledger', 'entry', 'fetch', 'contract-data', '--contract', address,
                           '--instance', *self.net, '--output', 'json'])
        data = json.loads(result.stdout)
        (self.output / f'instance-{address}.json').write_text(json.dumps(data, indent=2) + '\n')
        return data['entries'][0]['val']['contract_data']['val']['contract_instance']

    def sign_auth(self, assembled, expected):
        envelope = decode('TransactionEnvelope', assembled)
        entries = checked_invocation(envelope, self.payer, expected)['auth']
        for entry in entries:
            if entry['credentials'] == 'source_account':
                continue
            require('invoke_contract' in expected and entry['root_invocation']['function'] ==
                    {'contract_fn': expected['invoke_contract']}, 'unexpected owner authorization root')
            credential = entry['credentials']['address']
            require(credential['address'] == self.owner, 'unexpected required signer')
            credential['signature_expiration_ledger'] = self.rpc('getLatestLedger')['result']['sequence'] + 60
            preimage = {'soroban_authorization': {
                'network_id': hashlib.sha256(PASSPHRASE.encode()).hexdigest(),
                'nonce': credential['nonce'],
                'signature_expiration_ledger': credential['signature_expiration_ledger'],
                'invocation': entry['root_invocation']}}
            encoded = codec('encode', 'HashIdPreimage', json.dumps(preimage))
            message = hashlib.sha256(base64.b64decode(encoded, validate=True)).digest()
            # OpenSSL is the platform Ed25519 implementation; keys never enter evidence/logs.
            secret = self.cli(['keys', 'secret', 'owner']).stdout.strip()
            seed = base64.b32decode(secret)
            require(seed[0] == 144 and len(seed) == 35
                    and binascii.crc_hqx(seed[:-2], 0).to_bytes(2, 'little') == seed[-2:], 'invalid seed StrKey')
            key_file = self.private / 'auth.der'
            key_file.write_bytes(bytes.fromhex('302e020100300506032b657004220420') + seed[1:33])
            key_file.chmod(0o600)
            message_file = self.private / 'auth-message.bin'
            message_file.write_bytes(message)
            try:
                signature = subprocess.check_output(['openssl', 'pkeyutl', '-sign', '-rawin', '-keyform', 'DER',
                                                     '-inkey', str(key_file), '-in', str(message_file)],
                                                    stderr=subprocess.PIPE, timeout=30)
            finally:
                key_file.unlink(missing_ok=True)
            require(len(signature) == 64, 'invalid Ed25519 signature length')
            public = base64.b32decode(self.owner)[1:33].hex()
            credential['signature'] = {'vec': [{'map': [
                {'key': {'symbol': 'public_key'}, 'val': {'bytes': public}},
                {'key': {'symbol': 'signature'}, 'val': {'bytes': signature.hex()}}]}]}
        return codec('encode', 'TransactionEnvelope', json.dumps(envelope))

    def mutate(self, label, args):
        require(not self.report['unknown_outcome'], 'mutation blocked by unknown transaction outcome')
        built = self.cli([*args[:args.index('--')], '--build-only', *args[args.index('--'):]]
                         if '--' in args else [*args, '--build-only']).stdout.strip()
        expected = decode('TransactionEnvelope', built)['tx']['tx']['operations'][0]['body']['invoke_host_function']['host_function']
        self.index += 1
        stem = self.output / f'{self.index:03d}-{label}'
        stem.with_suffix('.request.json').write_text(json.dumps(expected, indent=2) + '\n')
        assembled = self.cli(['tx', 'simulate', *self.net, '--source', 'payer',
                              '--instruction-leeway', '20000000'], stdin=built).stdout.strip()
        authorized = self.sign_auth(assembled, expected)
        enforced = self.cli(['tx', 'simulate', *self.net, '--source', 'payer', '--auth-mode', 'enforce',
                             '--instruction-leeway', '20000000'], stdin=authorized).stdout.strip()
        signed = self.cli(['tx', 'sign', *self.net, '--sign-with-key', 'payer'], stdin=enforced).stdout.strip()
        final = checked_invocation(decode('TransactionEnvelope', signed), self.payer, expected)
        original = checked_invocation(decode('TransactionEnvelope', authorized), self.payer, expected)
        require(final.get('auth', []) == original.get('auth', []), 'enforced simulation changed authorization')
        tx_hash = self.cli(['tx', 'hash', '--network-passphrase', PASSPHRASE], stdin=signed).stdout.strip()
        require(re.fullmatch('[0-9a-f]{64}', tx_hash) is not None, 'invalid transaction hash')
        item = {'label': label, 'hash': tx_hash, 'status': 'UNKNOWN'}
        self.report['transactions'].append(item)
        self.report['unknown_outcome'] = True
        stem.with_suffix('.signed.xdr').write_text(signed + '\n')
        self.save()
        # The hash is durable before submission. Send this envelope once, then reconcile it.
        with stem.with_suffix('.stdout').open('w') as out, stem.with_suffix('.stderr').open('w') as err:
            try:
                subprocess.run(['stellar', '--config-dir', str(self.private / 'owner'), '--no-cache',
                                'tx', 'send', *self.net], input=signed, text=True, stdout=out, stderr=err,
                               timeout=180, env=self.env, cwd=ROOT)
            except subprocess.TimeoutExpired:
                pass
        for _ in range(20):
            try:
                receipt = self.rpc('getTransaction', {'hash': tx_hash})
                if receipt['result']['status'] in {'SUCCESS', 'FAILED'}:
                    break
            except (OSError, ValueError) as error:
                item['poll_error'] = str(error)
            time.sleep(3)
        else:
            raise RuntimeError(f'{label}: unknown transaction {tx_hash}; dependent mutations blocked')
        stem.with_suffix('.receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
        item['status'] = receipt['result']['status']
        self.save()
        if item['status'] == 'FAILED':
            verify(receipt, tx_hash, PASSPHRASE, self.payer, expected, status='FAILED')
            self.report['unknown_outcome'] = False
            self.save()
            raise ValueError(f'{label}: committed transaction failed: {tx_hash}')
        value = verify(receipt, tx_hash, PASSPHRASE, self.payer, expected)
        item['return'] = value
        self.report['unknown_outcome'] = False
        self.save()
        return value

    def invoke(self, contract, method, **values):
        return self.mutate(method, ['contract', 'invoke', *self.net, '--id', contract,
                                  '--source', 'payer', '--auto-sign',
                                  '--instruction-leeway', '20000000', '--send=yes', '--', method, *arguments(values)])

    def balance(self, address):
        return int(self.view(self.config['asset'], 'balance', id=address))

    def setup(self):
        require(self.config['network_passphrase'] == PASSPHRASE, 'refusing non-testnet configuration')
        network = self.rpc('getNetwork')['result']
        validate_network(self.config, network)
        self.report['network'] = network
        self.report['stellar'] = self.cli(['--version']).stdout.strip()
        require(self.report['stellar'].startswith('stellar 28.0.0 '), 'requires pinned stellar-cli 28.0.0')
        self.report['commit'] = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        self.report['source_sha256'] = {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest()
                                       for path in [*Path(__file__).parent.glob('*.py'), Path(__file__).with_name('testnet.json')]}
        self.report['diff_sha256'] = hashlib.sha256(subprocess.check_output(['git', 'diff', 'HEAD'], cwd=ROOT)).hexdigest()
        self.case('network')
        for role, names in [('owner', ('owner', 'payer')), ('attacker', ('attacker',))]:
            (self.private / role).mkdir(mode=0o700)
            for name in names:
                self.cli(['keys', 'generate', name], role)
                address = self.cli(['keys', 'address', name], role).stdout.strip()
                setattr(self, name, address)
                funded = subprocess.run(['curl', '-q', '-fsS', '--max-time', '60',
                                         'https://friendbot.stellar.org/?addr=' + address],
                                        capture_output=True, text=True, timeout=70)
                require(funded.returncode == 0, f'Friendbot failed for {name}: {funded.stderr}')
                (self.output / f'friendbot-{name}.json').write_text(funded.stdout)
                require(json.loads(funded.stdout).get('successful') is True, f'Friendbot did not fund {name}')
        self.report['wallets'] = {'owner': self.owner, 'payer': self.payer, 'attacker': self.attacker}
        controller = self.instance(self.config['controller'])
        storage = {entry['key']['vec'][0]['symbol']: entry['val'] for entry in controller['storage']}
        for key, name in [('Pool', 'pool'), ('PositionNft', 'position_nft')]:
            require(storage[key]['address'] == self.config[name], f'controller {name} changed')
        require(storage.get('Paused', {}).get('bool') is False, 'controller paused')
        self.aggregator = storage['PriceAggregator']['address']
        self.lending = {key: self.config[key] for key in ('controller', 'pool', 'position_nft')}
        self.market = {'hub_id': self.config['hub_id'], 'asset': self.config['asset']}
        self.controller, self.nft = self.config['controller'], self.config['position_nft']
        self.report['protocol_hashes'] = {name: self.instance(address)['executable']['wasm']
                                        for name, address in {**self.lending, 'price_aggregator': self.aggregator}.items()}
        native = self.cli(['contract', 'id', 'asset', '--asset', 'native', *self.net]).stdout.strip()
        require(native == self.config['asset'], 'expected the testnet native XLM SAC')
        flags = self.view(self.controller, 'get_spoke_asset', spoke_id=self.config['spoke_id'], hub_asset=self.market)
        require(flags['is_collateralizable'] and flags['is_borrowable'] and not flags['paused'] and not flags['frozen'],
                'testnet market does not permit lifecycle')
        require(not self.view(self.controller, 'get_spoke', spoke_id=self.config['spoke_id'])['is_deprecated'], 'spoke deprecated')
        require(int(self.view(native, 'decimals')) == 7, 'unexpected XLM decimals')
        for name in EXAMPLES:
            wasm = self.wasm_dir / (name + '.wasm')
            digest = hashlib.sha256(wasm.read_bytes()).hexdigest()
            uploaded = self.mutate('upload-' + name, ['contract', 'upload', *self.net, '--source', 'payer',
                                                     '--wasm', str(wasm), '--optimize=false'])
            require(uploaded == digest, 'uploaded WASM hash differs from built artifact')
            constructor = {'lending': self.lending}
            if name != 'price_reader':
                constructor['owner'] = self.owner
            if name in {'account_basics', 'lending_vault'}:
                constructor.update(market=self.market, spoke_id=self.config['spoke_id'])
            address = self.mutate('deploy-' + name, ['contract', 'deploy', *self.net, '--source', 'payer',
                                                    '--wasm', str(wasm), '--optimize=false', '--salt', secrets.token_hex(32), '--', *arguments(constructor)])
            self.report['contracts'][name] = {'address': address, 'wasm_sha256': digest}
            require(self.instance(address)['executable']['wasm'] == digest, 'deployed WASM differs from build')
            setattr(self, name, address)
        self.case('deployment')

    def owned(self, contract, account_id):
        require(int(account_id) > 0, 'invalid new account id')
        require(self.view(self.nft, 'owner_of', token_id=account_id) == contract, 'NFT belongs to wrong owner')
        require(int(self.view(self.controller, 'get_account_attributes', account_id=account_id)['spoke_id'])
                == self.config['spoke_id'], 'wrong spoke')
        require(self.balance(contract) == 0, 'wrapper retained deposited funds')

    def track(self, contract, account_id, close_args, principal):
        entry = {'contract': contract, 'account_id': account_id, 'close_args': close_args, 'principal': principal}
        self.report['open_accounts'].append(entry)
        self.save()
        self.owned(contract, account_id)
        supplied = int(self.view(self.controller, 'get_collateral_amount', account_id=account_id, hub_asset=self.market))
        require(supplied >= principal - 2, 'deposit credited too little collateral')
        return entry

    def close(self, entry):
        contract, account_id = entry['contract'], entry['account_id']
        before = self.balance(self.owner)
        result = self.invoke(contract, 'withdraw_all', **entry['close_args'])
        require(result['account_closed'] is True and int(result['amount']) >= entry['principal'] - 2, 'supply not fully recovered')
        require(self.balance(self.owner) - before == int(result['amount']), 'withdrawal payout differs from receipt')
        require(not self.view(self.controller, 'account_exists', account_id=account_id), 'account survived close')
        require(int(self.view(self.nft, 'balance', account=contract)) == 0, 'NFT not burned')
        self.report['open_accounts'].remove(entry)
        self.save()
        require(self.balance(contract) == 0, 'wrapper retained withdrawal funds')

    def negative_auth(self, contract, method, **values):
        before = [self.balance(address) for address in (self.owner, contract)]
        built = self.cli(['contract', 'invoke', *self.net, '--source', self.attacker, '--id', contract,
                          '--build-only', '--', method, *arguments(values)], 'attacker').stdout.strip()
        result = self.rpc('simulateTransaction', {'transaction': built, 'authMode': 'enforce'})['result']
        self.index += 1
        (self.output / f'{self.index:03d}-enforced-auth-{method}.json').write_text(json.dumps(result, indent=2) + '\n')
        require(re.search(r'Error\(Auth, (InvalidAction|InvalidInput)\)', result.get('error', '')) is not None,
                f'expected enforced authorization rejection, got {result.get("error")}')
        require(before == [self.balance(address) for address in (self.owner, contract)], 'authorization rejection changed funds')

    def scenarios(self):
        for _ in range(3):
            quote = self.view(self.price_reader, 'quote', asset=self.config['asset'])
            require(quote['valid'] and not quote['stale'] and not quote['deviation'] and quote['error_code'] is None,
                    'shared testnet oracle unusable: ' + str(quote))
            prices = [int(self.view(self.price_reader, method, asset=self.config['asset']))
                      for method in ('price', 'safe_price')]
            value = int(self.view(self.price_reader, 'value_usd', asset=self.config['asset'], amount=10_000_000))
            direct = self.view(self.aggregator, 'quotes', keys=[{'Token': self.config['asset']}])
            after = self.view(self.price_reader, 'quote', asset=self.config['asset'])
            if quote == after and list(direct.values()) == [quote]:
                expected = int(quote['final_wad'])
                require(expected > 0 and prices == [expected, expected] and value == expected,
                        'SDK strict/safe price or 7-decimal valuation differs from direct oracle')
                break
        else:
            raise ValueError('oracle changed during every price comparison; rerun when stable')
        self.case('prices')
        principal, borrow = 1_000_000_000, 50_000_000
        before = self.balance(self.owner)
        account_id = self.invoke(self.account_basics, 'deposit', **{'from': self.owner}, amount=principal)
        entry = self.track(self.account_basics, account_id, {}, principal)
        require(before - self.balance(self.owner) == principal, 'deposit debit mismatch')
        before = self.balance(self.owner)
        require(self.invoke(self.account_basics, 'deposit', **{'from': self.owner}, amount=10_000_000) == account_id,
                'top-up opened another account')
        require(before - self.balance(self.owner) == 10_000_000 and self.balance(self.account_basics) == 0, 'top-up debit/residue mismatch')
        entry['principal'] += 10_000_000
        supplied = int(self.view(self.controller, 'get_collateral_amount', account_id=account_id, hub_asset=self.market))
        require(supplied >= entry['principal'] - 2, 'top-up credit mismatch')
        self.negative_auth(self.account_basics, 'borrow', market=self.market, amount=borrow)
        before = self.balance(self.owner)
        self.invoke(self.account_basics, 'borrow', market=self.market, amount=borrow)
        require(self.balance(self.owner) - before == borrow and self.balance(self.account_basics) == 0, 'borrow payout mismatch')
        position = self.view(self.account_basics, 'position')
        require(len(position['collateral']) == 1 and int(position['collateral'][0][1]) >= entry['principal'] - 2, 'SDK collateral missing')
        require(len(position['debt']) == 1 and borrow <= int(position['debt'][0][1]) <= borrow + borrow // 100, 'SDK debt missing')
        require(int(self.view(self.account_basics, 'health_factor')) > 10**18, 'account unhealthy')
        require(int(self.view(self.price_reader, 'max_borrow', account_id=account_id, asset=self.config['asset'])) > 0,
                'missing borrow headroom')
        require(int(self.invoke(self.liquidator, 'liquidate', account_id=account_id, market=self.market, amount=borrow)) == 0,
                'healthy account liquidated')
        self.case('healthy_liquidation')
        self.repay(account_id)
        self.close(entry)
        require(self.view(self.account_basics, 'account_id') is None, 'closed id retained')
        self.case('account_lifecycle')
        reopened = self.invoke(self.account_basics, 'deposit', **{'from': self.owner}, amount=10_000_000)
        entry = self.track(self.account_basics, reopened, {}, 10_000_000)
        require(reopened != account_id, 'closed account id reused')
        self.close(entry)
        self.case('reopen')
        for name in ('simple_deposit', 'market_picker', 'lending_vault'):
            contract = getattr(self, name)
            values = {'from': self.owner, 'amount': principal}
            close_args = {}
            method = 'deposit'
            if name == 'simple_deposit':
                values.update(token=self.config['asset'], hub_id=self.config['hub_id'], spoke_id=self.config['spoke_id'], account_id=0)
            if name == 'market_picker':
                method = 'supply_best'
                values.update(asset=self.config['asset'], hub_ids=[self.config['hub_id']], spoke_ids=[self.config['spoke_id']])
                require(self.view(contract, 'pick', asset=self.config['asset'], hub_ids=[self.config['hub_id']],
                                  spoke_ids=[self.config['spoke_id']]) == {'market': self.market, 'spoke_id': self.config['spoke_id']},
                        'unexpected placement')
                require(int(self.view(contract, 'market_info', market=self.market)['liquidity']) > 0, 'no market cash')
            before = self.balance(self.owner)
            account_id = self.invoke(contract, method, **values)
            if name == 'simple_deposit':
                close_args = dict(account_id=account_id, token=self.config['asset'], hub_id=self.config['hub_id'])
            elif name == 'market_picker':
                close_args = dict(account_id=account_id, market=self.market)
            entry = self.track(contract, account_id, close_args, principal)
            require(before - self.balance(self.owner) == principal, 'deposit debit mismatch')
            self.negative_auth(contract, 'withdraw_all', **close_args)
            if name == 'lending_vault':
                require(int(self.view(contract, 'balance')) >= principal - 1, 'vault balance missing')
                self.flash()
            self.close(entry)
            self.case(name)
        amount = 10_000_000
        self.invoke(self.config['asset'], 'transfer', **{'from': self.owner}, to=self.liquidator, amount=amount)
        self.negative_auth(self.liquidator, 'sweep', asset=self.config['asset'], amount=amount)
        before = self.balance(self.owner)
        self.invoke(self.liquidator, 'sweep', asset=self.config['asset'], amount=amount)
        require(self.balance(self.owner) - before == amount and self.balance(self.liquidator) == 0, 'sweep payout mismatch')
        self.case('sweep')
        self.case('authorization')

    def repay(self, account_id):
        debt = int(self.view(self.controller, 'get_borrow_amount', account_id=account_id, hub_asset=self.market))
        if debt:
            offered = debt + max(debt // 100, 10_000)
            before = self.balance(self.owner)
            repaid = int(self.invoke(self.account_basics, 'repay', market=self.market, amount=offered))
            require(0 < repaid <= offered and before - self.balance(self.owner) == repaid, 'repay/refund mismatch')
            require(self.balance(self.account_basics) == 0, 'repay left residue')
            require(self.view(self.account_basics, 'position')['debt'] == [], 'repay left debt')

    def flash(self):
        data = self.view(self.config['pool'], 'get_sync_data', hub_asset=self.market)
        fee_bps = int(data['params']['flashloan_fee'])
        amount = 10_000_000
        fee = max(1, (amount * fee_bps + 5_000) // 10_000)
        require(fee_bps > 0 and data['params']['is_flashloanable'], 'expected enabled flash loans with positive fee')
        # Exercise recovery of an unused fee before the successful flash.
        self.invoke(self.config['asset'], 'transfer', **{'from': self.owner}, to=self.lending_vault, amount=fee)
        self.negative_auth(self.lending_vault, 'sweep', asset=self.config['asset'], amount=fee)
        before = self.balance(self.owner)
        self.invoke(self.lending_vault, 'sweep', asset=self.config['asset'], amount=fee)
        require(self.balance(self.owner) - before == fee and self.balance(self.lending_vault) == 0, 'unused flash fee not recovered')
        self.invoke(self.config['asset'], 'transfer', **{'from': self.owner}, to=self.lending_vault, amount=fee)
        self.invoke(self.controller, 'flash_loan', caller=self.owner, asset=self.market,
                    amount=amount, receiver=self.lending_vault, data='00')
        require(self.balance(self.lending_vault) == 0, 'flash principal/fee not repaid')
        self.case('flash_loan')

    def cleanup(self):
        require(not self.report['unknown_outcome'], 'cleanup blocked: unknown transaction outcome')
        for name in ('liquidator', 'lending_vault'):
            if hasattr(self, name):
                contract = getattr(self, name)
                remaining = self.balance(contract)
                if remaining:
                    self.invoke(contract, 'sweep', asset=self.config['asset'], amount=remaining)
        for entry in self.report['open_accounts'][:]:
            if entry['contract'] == getattr(self, 'account_basics', None):
                self.repay(entry['account_id'])
            self.close(entry)
        self.report['residual_balances'] = {name: self.balance(getattr(self, name))
                                            for name in EXAMPLES if hasattr(self, name)}
        require(not any(self.report['residual_balances'].values()), 'cleanup left wrapper token balances')
        self.case('cleanup')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=ROOT / 'target' / ('testnet-e2e-' + time.strftime('%Y%m%d-%H%M%S')))
    parser.add_argument('--wasm-dir', type=Path, default=ROOT / 'target' / 'testnet-e2e-wasm')
    args = parser.parse_args()
    private = tempfile.mkdtemp(prefix='xoxno-sdk-e2e-')
    run = Run(json.loads(Path(__file__).with_name('testnet.json').read_text()), args.output.resolve(), args.wasm_dir.resolve(), private)
    try:
        run.setup()
        run.scenarios()
    except Exception as error:
        run.report['errors'].append(str(error))
        print('FAIL:', error, file=sys.stderr, flush=True)
    finally:
        cleanup_failed = False
        try:
            run.cleanup()
        except Exception as error:
            cleanup_failed = True
            run.report['errors'].append('cleanup: ' + str(error))
        if cleanup_failed or run.report['open_accounts'] or run.report['unknown_outcome'] or any(run.report.get('residual_balances', {}).values()):
            run.report['private_state_dir'] = private
            print('Private recovery keys retained locally:', private, file=sys.stderr, flush=True)
        else:
            shutil.rmtree(private)
        run.report['status'] = final_status(run.report)
        run.save()
        print(run.report['status'], args.output / 'report.json', flush=True)
    return 0 if run.report['status'] == 'PASS' else 1


if __name__ == '__main__':
    sys.exit(main())
