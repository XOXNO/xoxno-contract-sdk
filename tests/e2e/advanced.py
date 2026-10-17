#!/usr/bin/env python3
"""Exercise SDK batches and real routed strategies on the shared lending testnet."""
import base64
import json
import sys
import time
import urllib.parse
import urllib.request

from receipts import codec, decode
from run import Run, arguments, main, require

UNIT = 10_000_000
USDC = 'CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA'
ROUTER = 'CDNTWMWW2WGYTKIZTJYNGNVQQZI4KTC5BQRZ3275KESRX5T4O3AYECL5'
QUOTE_API = 'https://testnet-stellar-swap.xoxno.com/api/v1'
BLEND = 'CCEBVDYM32YNYCVNRXQKDFFPISJJCV557CDZEIRBEE4NCV4KHPQ44HGF'


def key(asset, hub=1):
    return {'hub_id': hub, 'asset': asset}


def payments(*items):
    return [[market, str(amount)] for market, amount in items]


def positions(rows):
    return {(int(market['hub_id']), market['asset']): int(amount) for market, amount in rows}


def validate_quote(quote, source, dest, amount, latest):
    require(quote['from'] == source and quote['to'] == dest and int(quote['amountIn']) == amount,
            'quote input differs')
    require(int(quote['amountOutMin']) > 0 and quote['routeXdr'], 'empty or unbounded route')
    ledger = int(quote['snapshot']['ledger'])
    require(0 < ledger <= latest and latest - ledger <= 12, 'quote ledger stale or future')
    return base64.b64decode(quote['routeXdr'], validate=True).hex()


def invalid_route_version(route):
    payload = decode('ScVal', base64.b64encode(bytes.fromhex(route)).decode())
    ops = next(row['val'] for row in payload['map'] if row['key'] == {'symbol':'ops'})
    require(ops['bytes'].startswith('01'), 'expected version-one control route')
    ops['bytes'] = 'ff' + ops['bytes'][2:]
    return base64.b64decode(codec('encode', 'ScVal', json.dumps(payload)), validate=True).hex()


class AdvancedRun(Run):
    examples = ('advanced_account',)
    required = {'network', 'deployment', 'funding', 'batch_lifecycle', 'batch_duplicates',
                'batch_cross_hub', 'batch_rejections', 'batch_healthy_liquidation',
                'multiply_long', 'multiply_short', 'multiply', 'swap_debt', 'swap_collateral',
                'repay_collateral', 'repay_collateral_close', 'same_market_settlement',
                'flash_position', 'strategy_rejections', 'authorization', 'cleanup'}

    def __init__(self, config, *args):
        config = {**config, 'spoke_id': 3}
        super().__init__(config, *args)
        self.tokens = [config['asset'], USDC]
        self.markets = [key(config['asset']), key(USDC), key(USDC, 2)]

    def constructor(self, name):
        return dict(owner=self.owner, lending=self.lending, router=ROUTER, initiator=self.owner)

    def token_balance(self, asset, address):
        return int(self.view(asset, 'balance', id=address))

    def wallet(self):
        return {asset: self.token_balance(asset, self.owner) for asset in self.tokens}

    def pos(self, account):
        return self.view(self.advanced_account, 'position', account_id=account)

    def snapshot(self, account=0):
        exists = bool(account and self.view(self.controller, 'account_exists', account_id=account))
        return {'balances': {asset: {address: self.token_balance(asset, address)
                                    for address in [self.owner, self.advanced_account, self.controller,
                                                    self.config['pool'], getattr(self, 'receiver', self.advanced_account)]}
                             for asset in self.tokens},
                'nft_balance': self.view(self.nft, 'balance', account=self.advanced_account),
                'exists': exists,
                'positions': self.view(self.controller, 'get_account_positions', account_id=account) if exists else None,
                'owner': self.view(self.nft, 'owner_of', token_id=account) if exists else None,
                'attributes': self.view(self.controller, 'get_account_attributes', account_id=account) if exists else None}

    def reject(self, method, account=0, auth=False, pattern=None, **values):
        before = self.snapshot(account)
        built = self.cli(['contract', 'invoke', *self.net, '--source', self.attacker if auth else self.payer,
                          '--id', self.advanced_account, '--build-only', '--', method, *arguments(values)],
                         'attacker' if auth else 'owner').stdout.strip()
        result = self.rpc('simulateTransaction', {'transaction': built, **({'authMode': 'enforce'} if auth else {})})['result']
        self.index += 1
        (self.output / f'{self.index:03d}-rejected-{method}.json').write_text(json.dumps(result, indent=2) + '\n')
        error = result.get('error', '')
        require((('Error(Auth,' if auth else 'Error(Contract,') in error) or
                (not auth and pattern == 'Error(WasmVm, InvalidAction)' and pattern in error),
                'expected contract/auth rejection: ' + error)
        if pattern:
            require(pattern in error, 'wrong rejection: ' + error)
        require(before == self.snapshot(account), 'rejected operation changed balances, NFT or principal')

    def route(self, source, dest, amount):
        query = urllib.parse.urlencode({
            'from': source, 'to': dest, 'amount_in': amount, 'slippage': '0.05', 'max_splits': 1, 'max_hops': 2})
        with urllib.request.urlopen(QUOTE_API + '/quote?' + query, timeout=40) as response:
            quote = json.load(response)
        self.index += 1
        (self.output / f'{self.index:03d}-quote.json').write_text(json.dumps(quote, indent=2) + '\n')
        return validate_quote(quote, source, dest, amount, self.rpc('getLatestLedger')['result']['sequence'])

    def trustline(self, asset):
        name = self.view(asset, 'name')
        require(':' in name, 'expected issued-asset SAC')
        self.classic('change-trust', line=name)

    def open(self, method='deposit_batch', **values):
        account = int(self.invoke(self.advanced_account, method, **values))
        require(0 < account <= 2**32 - 1, 'invalid NFT account id')
        self.report['open_accounts'].append({'contract': self.advanced_account, 'account_id': account})
        self.save()
        require(self.view(self.nft, 'owner_of', token_id=account) == self.advanced_account, 'wrong NFT owner')
        return account

    def check_idle(self):
        for name in ('advanced_account', 'receiver'):
            if hasattr(self, name):
                for asset in self.tokens:
                    require(self.token_balance(asset, getattr(self, name)) == 0, name + ' retained tokens')

    def close_account(self, account):
        for market, amount in self.pos(account)['debt']:
            offered = int(amount) + max(100_000, int(amount) // 100)
            before = self.token_balance(market['asset'], self.owner)
            repaid = self.invoke(self.advanced_account, 'repay_batch', account_id=account,
                                 assets=payments((market, offered)))
            require(len(repaid) == 1 and 0 < int(repaid[0][1]) <= offered, 'invalid repayment receipt')
            require(before - self.token_balance(market['asset'], self.owner) == int(repaid[0][1]), 'repay refund mismatch')
        require(self.pos(account)['debt'] == [], 'repay left debt')
        collateral = self.pos(account)['collateral']
        if collateral:
            before = self.wallet()
            result = self.invoke(self.advanced_account, 'withdraw_batch', account_id=account,
                                 assets=payments(*[(market, 0) for market, _ in collateral]))
            totals = dict.fromkeys(self.tokens, 0)
            for market, amount in result['amounts']:
                totals[market['asset']] += int(amount)
            require(result['account_closed'] and all(self.wallet()[t] - before[t] == totals[t] for t in self.tokens),
                    'withdraw/refund mismatch')
        require(not self.view(self.controller, 'account_exists', account_id=account), 'account survived close')
        self.report['open_accounts'] = [entry for entry in self.report['open_accounts'] if entry['account_id'] != account]
        self.save()
        self.check_idle()

    def scenarios(self):
        xlm, usd, usd2 = self.markets
        for market in self.markets:
            flags = self.view(self.controller, 'get_spoke_asset', spoke_id=3, hub_asset=market)
            require(flags['is_collateralizable'] and flags['is_borrowable'] and not flags['paused'] and not flags['frozen'],
                    'advanced market disabled')
        self.trustline(USDC)
        amount = 300 * UNIT
        route = self.route(xlm['asset'], USDC, amount)
        before = self.wallet()
        received = self.invoke(ROUTER, 'execute_strategy', sender=self.owner, total_in=amount, swap_xdr=route)
        require(before[xlm['asset']] - self.wallet()[xlm['asset']] == amount
                and self.wallet()[USDC] - before[USDC] > 0 and received, 'funding swap failed')
        self.case('funding')
        # Same token in two hubs remains two independent market positions.
        assets = payments((xlm, 100 * UNIT), (usd, 20 * UNIT), (usd2, 10 * UNIT), (usd, 5 * UNIT))
        before = self.wallet()
        account = self.open(account_id=0, spoke_id=3, assets=assets)
        p = positions(self.pos(account)['collateral'])
        require(len(p) == 3 and p[(1, USDC)] >= 25 * UNIT - 2 and p[(2, USDC)] >= 10 * UNIT - 2, 'batch market merge wrong')
        require(before[xlm['asset']] - self.wallet()[xlm['asset']] == 100 * UNIT
                and before[USDC] - self.wallet()[USDC] == 35 * UNIT, 'batch token debit wrong')
        before_supply = self.wallet()
        old_supply = p[(1, USDC)]
        require(int(self.invoke(self.advanced_account, 'supply_batch', account_id=account,
                            assets=payments((usd, UNIT), (usd, 2 * UNIT)))) == account, 'supply changed account')
        require(before_supply[USDC] - self.wallet()[USDC] == 3 * UNIT and
                positions(self.pos(account)['collateral'])[(1, USDC)] >= old_supply + 3 * UNIT - 2,
                'supply duplicate debit/credit wrong')
        self.case('batch_duplicates')
        borrows = payments((xlm, UNIT), (usd, UNIT), (usd2, UNIT))
        before = self.wallet()
        self.invoke(self.advanced_account, 'borrow_batch', account_id=account, assets=borrows)
        require(self.wallet()[xlm['asset']] - before[xlm['asset']] == UNIT and self.wallet()[USDC] - before[USDC] == 2 * UNIT,
                'cross-hub borrow payout wrong')
        require(len(self.pos(account)['debt']) == 3, 'cross-hub debt collapsed')
        self.reject('repay_batch', account, account_id=account, assets=payments((usd, 2 * UNIT), (usd2, 2 * UNIT)), pattern='#16')
        self.case('batch_cross_hub')
        debt_before = positions(self.pos(account)['debt'])[(1, USDC)]
        wallet_before = self.wallet()
        repaid = self.invoke(self.advanced_account, 'repay_batch', account_id=account, assets=payments((usd, UNIT // 4), (usd, UNIT // 4)))
        require(len(repaid) == 1 and int(repaid[0][1]) == UNIT // 2 and
                wallet_before[USDC] - self.wallet()[USDC] == UNIT // 2 and
                positions(self.pos(account)['debt'])[(1, USDC)] < debt_before - UNIT // 3,
                'duplicate repayment debit/credit wrong')
        wallet_before = self.wallet()
        collateral_before = positions(self.pos(account)['collateral'])
        withdrawn = self.invoke(self.advanced_account, 'withdraw_batch', account_id=account, assets=payments((xlm, UNIT), (usd, UNIT)))
        require(not withdrawn['account_closed'] and len(withdrawn['amounts']) == 2, 'partial withdraw wrong result')
        for market, amount in withdrawn['amounts']:
            require(int(amount) == UNIT and self.wallet()[market['asset']] - wallet_before[market['asset']] == UNIT and
                    positions(self.pos(account)['collateral'])[(1, market['asset'])] < collateral_before[(1, market['asset'])],
                    'partial withdrawal payout/principal wrong')
        self.invoke(self.advanced_account, 'renew_account', account_id=account)
        for amount in (0, -1):
            self.reject('deposit_batch', account, account_id=account, spoke_id=3, assets=payments((usd, amount)),
                        pattern='Error(WasmVm, InvalidAction)')
        self.reject('borrow_batch', account, account_id=account, assets=payments((usd, 10**15)))
        self.reject('withdraw_batch', account, account_id=account, assets=payments((xlm, 0), (usd, 0), (usd2, 0)))
        self.reject('deposit_batch', account, account_id=account, spoke_id=3, assets=payments((key(USDC, 999), UNIT)))
        self.reject('withdraw_batch', account, auth=True, account_id=account, assets=payments((usd, UNIT)))
        self.case('batch_rejections')
        self.reject('liquidate_batch', account, victim=account,
                    assets=payments((xlm, UNIT), (usd, UNIT)), pattern='#101')
        self.case('batch_healthy_liquidation')
        self.close_account(account)
        self.case('batch_lifecycle')
        for mode, label in ((1, 'multiply'), (2, 'multiply_long'), (3, 'multiply_short')):
            debt, coll = (usd, xlm) if mode != 3 else (xlm, usd)
            draw = UNIT
            route = self.route(debt['asset'], coll['asset'], draw)
            request = dict(account_id=0, spoke_id=3, collateral=coll, debt=debt, amount=str(draw), mode=mode,
                           swap=route, initial=payments((coll, 100 * UNIT)), convert=None)
            account = self.open('multiply', request=request)
            p = self.pos(account)
            require(positions(p['debt'])[(1, debt['asset'])] >= draw and positions(p['collateral'])[(1, coll['asset'])] > 100 * UNIT,
                    'multiply principal missing')
            self.case(label)
            if mode == 1:
                self.strategies(account, xlm, usd)
            self.close_account(account)
        self.same_market(xlm)
        self.flash_position(xlm, usd)
        self.case('authorization')

    def strategies(self, account, xlm, usd):
        before = positions(self.pos(account)['debt'])[(1, USDC)]
        route = self.route(xlm['asset'], USDC, UNIT)
        self.invoke(self.advanced_account, 'swap_debt', request=dict(account_id=account, current=usd, next=xlm, amount=str(UNIT), swap=route))
        p = self.pos(account)
        require(positions(p['debt'])[(1, xlm['asset'])] >= UNIT and positions(p['debt']).get((1, USDC), 0) < before,
                'debt swap wrong direction')
        self.case('swap_debt')
        before = positions(p['collateral'])[(1, xlm['asset'])]
        self.invoke(self.advanced_account, 'swap_collateral', request=dict(account_id=account, current=xlm, next=usd,
                    amount=str(UNIT), swap=self.route(xlm['asset'], USDC, UNIT)))
        p = positions(self.pos(account)['collateral'])
        require(p[(1, xlm['asset'])] < before and p[(1, USDC)] > 0, 'collateral swap wrong direction')
        self.case('swap_collateral')
        self.invoke(self.advanced_account, 'borrow_batch', account_id=account, assets=payments((usd, 2 * UNIT)))
        before = positions(self.pos(account)['debt'])[(1, USDC)]
        self.invoke(self.advanced_account, 'repay_collateral', request=dict(account_id=account, collateral=xlm, debt=usd,
                    amount=str(UNIT), swap=self.route(xlm['asset'], USDC, UNIT), close=False))
        require(positions(self.pos(account)['debt']).get((1, USDC), 0) < before, 'collateral repayment did not reduce debt')
        self.case('repay_collateral')
        self.reject('swap_collateral', account, pattern='Error(Contract, #13)',
                    request=dict(account_id=account, current=xlm, next=usd, amount=str(UNIT), swap=invalid_route_version(route)))
        self.reject('swap_debt', account, auth=True, request=dict(account_id=account, current=usd, next=xlm, amount=str(UNIT), swap='00'))
        self.case('strategy_rejections')

    def same_market(self, market):
        account = self.open(account_id=0, spoke_id=3, assets=payments((market, 100 * UNIT)))
        self.invoke(self.advanced_account, 'borrow_batch', account_id=account, assets=payments((market, 2 * UNIT)))
        request = dict(account_id=account, collateral=market, debt=market, amount=str(UNIT), swap='', close=False)
        self.invoke(self.advanced_account, 'repay_collateral', request=request)
        require(0 < int(self.pos(account)['debt'][0][1]) < 2 * UNIT, 'same-market partial settlement failed')
        self.case('same_market_settlement')
        request.update(amount=str(100 * UNIT), close=True)
        self.invoke(self.advanced_account, 'repay_collateral', request=request)
        require(not self.view(self.controller, 'account_exists', account_id=account), 'same-market close left account')
        self.report['open_accounts'] = [entry for entry in self.report['open_accounts'] if entry['account_id'] != account]
        self.save()
        self.case('repay_collateral_close')

    def flash_position(self, xlm, usd):
        self.deploy('receiver', self.wasm_dir / 'advanced_account.wasm', dict(owner=self.owner, lending=self.lending,
                    router=ROUTER, initiator=self.advanced_account))
        self.invoke(USDC, 'transfer', **{'from': self.owner}, to=self.receiver, amount=20 * UNIT)
        route = self.route(xlm['asset'], USDC, UNIT)
        payload = {'map': [{'key': {'symbol': 'collateral'}, 'val': {'address': USDC}},
                           {'key': {'symbol': 'swap'}, 'val': {'bytes': route}}]}
        data = base64.b64decode(codec('encode', 'ScVal', json.dumps(payload)), validate=True).hex()
        account = self.open('flash_position', request=dict(account_id=0, spoke_id=3, mode=1, debt=xlm, amount=str(UNIT),
                    receiver=self.receiver, data=data, collaterals=payments((usd, 1)), refunds=[xlm['asset']]))
        p = self.pos(account)
        require(positions(p['collateral'])[(1, USDC)] > 20 * UNIT and positions(p['debt'])[(1, xlm['asset'])] >= UNIT,
                'flash callback did not back debt with received collateral')
        self.check_idle()
        self.case('flash_position')
        self.close_account(account)

    def cleanup(self):
        require(not self.report['unknown_outcome'], 'cleanup blocked by unknown outcome')
        if not hasattr(self, 'advanced_account'):
            self.case('cleanup')
            return
        for entry in self.report['open_accounts'][:]:
            if self.view(self.controller, 'account_exists', account_id=entry['account_id']):
                self.close_account(entry['account_id'])
            else:
                self.report['open_accounts'].remove(entry)
        residual = {}
        for name in ('advanced_account', 'receiver'):
            if hasattr(self, name):
                contract = getattr(self, name)
                for asset in self.tokens:
                    if self.token_balance(asset, contract):
                        self.invoke(contract, 'sweep', asset=asset)
                    residual[name + ':' + asset] = self.token_balance(asset, contract)
                require(int(self.view(self.nft, 'balance', account=contract)) == 0, 'cleanup left NFT')
        self.report['residual_balances'] = residual
        require(not any(residual.values()), 'cleanup left funds')
        self.case('cleanup')


if __name__ == '__main__':
    sys.exit(main(AdvancedRun))
