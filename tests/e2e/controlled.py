#!/usr/bin/env python3
"""Fresh isolated testnet protocol: controlled liquidation and real Blend migration."""
import hashlib
import json
import secrets
import sys
import time

from advanced import AdvancedRun, BLEND, ROUTER, UNIT, USDC, key, payments, positions
from receipts import codec, decode
from run import ROOT, arguments, main, require

WAD = 10**18
RAY = 10**27


class ControlledRun(AdvancedRun):
    examples = ()
    required = {'network', 'deployment', 'isolated_protocol', 'funding', 'liquidation_batch',
                'blend_migration', 'blend_refunds', 'cleanup'}

    def upload(self, name):
        wasm = ROOT / 'crates/xoxno-contract-sdk/wasm/deploy' / (name + '.wasm')
        digest = hashlib.sha256(wasm.read_bytes()).hexdigest()
        result = self.mutate('upload-' + name, ['contract', 'upload', *self.net, '--source', 'payer',
                             '--wasm', str(wasm), '--optimize=false'])
        require(result == digest, 'fixture WASM hash mismatch')
        return digest

    def govern(self, op, target, method, **values):
        require(target in self.owned_protocol, 'refusing governance against shared deployment')
        salt = secrets.token_hex(32)
        operation = self.invoke(self.governance, 'propose', proposer=self.owner, op=json.dumps(op) if isinstance(op, str) else op, salt=salt)
        ready = int(self.view(self.governance, 'get_operation_ledger', operation_id=operation))
        deadline = time.monotonic() + 60
        while self.rpc('getLatestLedger')['result']['sequence'] < ready:
            require(time.monotonic() < deadline, 'timelock did not advance')
            time.sleep(2)
        built = self.cli(['contract', 'invoke', *self.net, '--source', 'payer', '--id', target,
                          '--build-only', '--', method, *arguments(values)]).stdout.strip()
        envelope = decode('TransactionEnvelope', built)
        invocation = envelope['tx']['tx']['operations'][0]['body']['invoke_host_function']
        target_args = invocation['host_function']['invoke_contract']['args']
        invocation['host_function'] = {'invoke_contract': {'contract_address': self.governance, 'function_name': 'execute',
            'args': ['void', {'address': target}, {'symbol': method}, {'vec': target_args}, {'bytes': '00'*32}, {'bytes': salt}]}}
        return self.mutate_built('execute-' + method, codec('encode', 'TransactionEnvelope', json.dumps(envelope)))

    def setup(self):
        super().setup()
        self.config['spoke_id'] = 1
        deploy_dir = ROOT / 'crates/xoxno-contract-sdk/wasm/deploy'
        self.deploy('governance', deploy_dir / 'governance.wasm', dict(admin=self.owner, min_delay=1))
        ctrl_hash, pool_hash, nft_hash, oracle_hash = [self.upload(name) for name in ('controller', 'pool', 'position_nft', 'price_aggregator')]
        self.controller = self.invoke(self.governance, 'deploy_controller', wasm_hash=ctrl_hash)
        self.aggregator = self.invoke(self.governance, 'deploy_price_aggregator', wasm_hash=oracle_hash)
        self.owned_protocol = {self.controller, self.aggregator}
        self.config['pool'] = self.govern({'DeployPool': pool_hash}, self.controller, 'deploy_pool', wasm_hash=pool_hash)
        nft = dict(wasm_hash=nft_hash, uri='https://test.xoxno.com/sdk/', name='SDK E2E', symbol='SDK')
        self.nft = self.govern({'DeployPositionNft': nft}, self.controller, 'deploy_position_nft', **nft)
        self.config['controller'], self.config['position_nft'] = self.controller, self.nft
        self.lending = dict(controller=self.controller, pool=self.config['pool'], position_nft=self.nft)
        hub = int(self.invoke(self.governance, 'create_hub', caller=self.owner))
        spoke = int(self.invoke(self.governance, 'add_spoke', caller=self.owner))
        require(hub == spoke == 1, 'unexpected isolated market ids')
        curve = dict(spoke_id=1, target_hf_wad=str(WAD*115//100), hf_for_max_bonus_wad=str(WAD*90//100), liquidation_bonus_factor_bps=1500)
        self.govern({'SetSpokeLiquidationCurve': curve}, self.controller, 'set_spoke_liquidation_curve', **{('id' if k == 'spoke_id' else k):v for k,v in curve.items()})
        self.govern('Unpause', self.controller, 'unpause')
        self.govern({'ApproveBlendPool': BLEND}, self.controller, 'approve_blend_pool', pool=BLEND)
        for name in ('mock_reflector', 'mock_redstone'):
            self.deploy(name, ROOT / 'crates/xoxno-contract-sdk/wasm' / (name+'.wasm'), {})
        self.markets = [key(self.config['asset']), key(USDC)]
        for market, symbol, price in zip(self.markets, ('XLM', 'USDC'), (WAD//5, WAD)):
            params = dict(asset_decimals=7, asset_id=market['asset'], max_borrow_rate=str(RAY//4), base_borrow_rate='0',
                slope1=str(RAY//40), slope2=str(RAY//40), slope3=str(RAY//5), mid_utilization=str(RAY*46//100),
                optimal_utilization=str(RAY*92//100), max_utilization=str(RAY*95//100), reserve_factor=1000,
                is_flashloanable=True, flashloan_fee=9)
            args = dict(hub_id=1, asset=market['asset'], params=params)
            self.govern({'CreateLiquidityPool': args}, self.controller, 'create_liquidity_pool', **args)
            risk = dict(asset=market['asset'], hub_id=1, spoke_id=1, bonus=400, borrow_cap=str(10**13), can_borrow=True,
                        can_collateral=True, frozen=False, liquidation_fees=1000, ltv=7600, no_seize=False, paused=False,
                        supply_cap=str(10**13), threshold=8000)
            self.govern({'AddAssetToSpoke': risk}, self.controller, 'add_asset_to_spoke', input=risk)
            self.set_price(market['asset'], symbol, price)
            tolerance = self.view(self.governance, 'resolve_oracle_tolerance', tolerance=500)
            oracle = dict(asset_decimals=0, independence='RequireDisjoint', max_price_stale_seconds=86400,
                          max_sanity_price_wad=str(10**9*WAD), min_sanity_price_wad='1', tolerance=tolerance,
                          sources=[{'Feed': dict(decimals=14, max_stale_seconds=86400, provider={'Reflector': dict(
                              asset={'Stellar':market['asset']}, contract=self.mock_reflector, read_mode={'Twap':3})})},
                                   {'Feed': dict(decimals=8, max_stale_seconds=86400, provider={'RedStone': dict(
                                       contract=self.mock_redstone, feed_id=symbol, nature='Fundamental')})}])
            pricekey = {'Token':market['asset']}
            resolved = self.view(self.governance, 'resolve_asset_oracle', key=pricekey, oracle=oracle)
            self.govern({'ConfigureAssetOracle':dict(key=pricekey, oracle=oracle)}, self.aggregator, 'set_oracle', key=pricekey, oracle=resolved)
        self.deploy('advanced_account', self.wasm_dir/'advanced_account.wasm', self.constructor('advanced_account'))
        self.deploy('liquidator', self.wasm_dir/'advanced_account.wasm', self.constructor('liquidator'))
        self.report['shared_protocol_hashes'] = self.report['protocol_hashes']
        expected = dict(controller=ctrl_hash, pool=pool_hash, position_nft=nft_hash, price_aggregator=oracle_hash)
        actual = {name:self.instance(address)['executable']['wasm'] for name,address in
                  {**self.lending, 'price_aggregator':self.aggregator}.items()}
        require(actual == expected, 'isolated protocol code differs from attested deployment WASM')
        self.report['protocol_hashes'] = actual
        self.report['isolated_protocol'] = {**self.lending, 'governance':self.governance, 'aggregator':self.aggregator,
                                           'wasm_source_manifest': json.loads((ROOT/'crates/xoxno-contract-sdk/wasm/MANIFEST.json').read_text())}
        self.save()
        self.case('isolated_protocol')

    def set_price(self, asset, symbol, price):
        require(self.mock_reflector in {c['address'] for c in self.report['contracts'].values()}, 'oracle not owned')
        self.invoke(self.mock_reflector, 'set_price', asset={'Stellar':asset}, price_wad=str(price))
        self.invoke(self.mock_redstone, 'set_price', feed_id=symbol, price_wad=str(price))

    def scenarios(self):
        xlm, usd = self.markets
        self.trustline(USDC)
        amount = 300*UNIT
        self.invoke(ROUTER, 'execute_strategy', sender=self.owner, total_in=amount, swap_xdr=self.route(xlm['asset'], USDC, amount))
        self.case('funding')
        liquidity = self.open(account_id=0, spoke_id=1, assets=payments((xlm, 400*UNIT), (usd, 100*UNIT)))
        victim = self.open(account_id=0, spoke_id=1, assets=payments((xlm, 100*UNIT)))
        self.invoke(self.advanced_account, 'borrow_batch', account_id=victim, assets=payments((usd, 12*UNIT), (xlm, UNIT)))
        self.set_price(xlm['asset'], 'XLM', WAD*14//100)
        before = self.wallet()
        victim_before = self.pos(victim)
        debt_before = positions(victim_before['debt'])
        result = self.invoke(self.liquidator, 'liquidate_batch', victim=victim, assets=payments((usd, UNIT//2), (xlm, UNIT), (usd, UNIT//2)))
        require(len(result) == 2, 'multi-token liquidation omitted a used leg')
        for market, spent in result:
            require(before[market['asset']] - self.wallet()[market['asset']] == int(spent), 'liquidation measured payout wrong')
        debt_after = positions(self.pos(victim)['debt'])
        require(all(debt_after.get(m,0) < amount for m,amount in debt_before.items()), 'multi-token liquidation failed to reduce each debt')
        require(self.wallet()[xlm['asset']] > before[xlm['asset']] and
                positions(self.pos(victim)['collateral'])[(1,xlm['asset'])] < positions(victim_before['collateral'])[(1,xlm['asset'])],
                'liquidation did not seize and deliver collateral')
        self.case('liquidation_batch')
        self.set_price(xlm['asset'], 'XLM', WAD//5)
        self.close_account(victim)
        # Real Blend owns the source position at the adapter address.
        requests = [dict(request_type=2, address=xlm['asset'], amount=str(100*UNIT)),
                    dict(request_type=0, address=xlm['asset'], amount=str(10*UNIT)),
                    dict(request_type=4, address=xlm['asset'], amount=str(UNIT))]
        self.report['blend_open'] = True
        self.save()
        self.invoke(self.advanced_account, 'blend_submit', pool=BLEND, requests=requests[:2])
        before = self.invoke(self.advanced_account, 'blend_submit', pool=BLEND, requests=requests[2:])
        require(before['collateral'] and before['supply'] and before['liabilities'], 'real Blend seed missing positions')
        wallet = self.wallet()
        account = self.open('migrate_blend', request=dict(account_id=0, spoke_id=1, hub_id=1, pool=BLEND,
                            collateral=[xlm['asset']], supply=[xlm['asset']], debts=[[xlm['asset'],str(2*UNIT)]]))
        old = self.view(BLEND, 'get_positions', address=self.advanced_account)
        require(not old['collateral'] and not old['supply'] and not old['liabilities'], 'Blend positions survived migration')
        self.report['blend_open'] = False
        p = self.pos(account)
        require(positions(p['collateral'])[(1,xlm['asset'])] >= 110*UNIT-1000 and
                UNIT <= positions(p['debt'])[(1,xlm['asset'])] < 2*UNIT, 'migrated principal wrong')
        self.case('blend_migration')
        require(0 <= self.wallet()[xlm['asset']] - wallet[xlm['asset']] <= 2 and
                positions(p['debt'])[(1,xlm['asset'])] < UNIT*101//100 and
                self.token_balance(xlm['asset'],self.advanced_account) == self.token_balance(xlm['asset'],self.controller) == 0,
                'unused debt cap did not reduce hub debt or left residue')
        self.case('blend_refunds')
        self.close_account(account)
        self.close_account(liquidity)

    def cleanup(self):
        require(not self.report['unknown_outcome'], 'cleanup blocked by unknown outcome')
        if hasattr(self, 'mock_reflector'):
            self.set_price(self.config['asset'], 'XLM', WAD//5)
        if self.report.get('blend_open'):
            old = self.view(BLEND, 'get_positions', address=self.advanced_account)
            if any(old.values()):
                reserves = self.view(BLEND, 'get_reserve_list')
                index = str(reserves.index(self.config['asset']))
                require(all(set(side) <= {index} for side in old.values()), 'unexpected Blend assets; retain keys')
                self.open('migrate_blend', request=dict(account_id=0, spoke_id=1, hub_id=1, pool=BLEND,
                          collateral=[self.config['asset']] if old['collateral'] else [],
                          supply=[self.config['asset']] if old['supply'] else [],
                          debts=[[self.config['asset'],str(10*UNIT)]] if old['liabilities'] else []))
                require(not any(self.view(BLEND, 'get_positions', address=self.advanced_account).values()),
                        'Blend recovery left source positions; retain keys')
            self.report['blend_open'] = False
        # Liquidity seeded first; close borrowers first.
        self.report['open_accounts'].reverse()
        super().cleanup()
        if hasattr(self, 'liquidator'):
            for asset in self.tokens:
                if self.token_balance(asset, self.liquidator):
                    self.invoke(self.liquidator, 'sweep', asset=asset)
                require(self.token_balance(asset, self.liquidator) == 0, 'liquidator retained token')


if __name__ == '__main__':
    sys.exit(main(ControlledRun))
