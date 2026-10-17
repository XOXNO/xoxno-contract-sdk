"""Verify committed single-operation receipts with Stellar CLI's XDR codec.

Derived from rs-lending-xlm/tests/integration/receipts.py; no Python dependencies.
"""
import base64
import hashlib
import json
import subprocess


def codec(action, kind, value):
    return subprocess.check_output(
        ['stellar', 'xdr', action, '--type', kind], input=value, text=True,
        stderr=subprocess.PIPE, timeout=30).strip()


def decode(kind, value):
    return json.loads(codec('decode', kind, value))


def plain(value):
    if value == 'void':
        return None
    if not isinstance(value, dict) or len(value) != 1:
        raise ValueError('invalid SCVal')
    kind, item = next(iter(value.items()))
    if kind in {'bool', 'u32', 'i32', 'u64', 'i64', 'u128', 'i128', 'address', 'string', 'symbol', 'bytes'}:
        return item
    if kind == 'vec':
        return [plain(entry) for entry in item]
    if kind == 'map':
        result = {}
        for entry in item:
            key = plain(entry['key'])
            if not isinstance(key, str) or key in result:
                raise ValueError('invalid or duplicate map key')
            result[key] = plain(entry['val'])
        return result
    raise ValueError('unsupported SCVal: ' + kind)


def verify(receipt, tx_hash, passphrase, source, expected_host, status='SUCCESS'):
    """Return the committed return value, never a simulation's prediction."""
    if receipt.get('jsonrpc') != '2.0' or receipt.get('id') != 1 or 'error' in receipt:
        raise ValueError('invalid JSON-RPC receipt')
    result = receipt['result']
    if (result['status'] != status or result['txHash'] != tx_hash
            or type(result['ledger']) is not int or result['ledger'] <= 0):
        raise ValueError('unconfirmed transaction')
    envelope = decode('TransactionEnvelope', result['envelopeXdr'])
    if set(envelope) != {'tx'}:
        raise ValueError('expected ordinary transaction envelope')
    payload = {'network_id': hashlib.sha256(passphrase.encode()).hexdigest(),
               'tagged_transaction': {'tx': envelope['tx']['tx']}}
    encoded = codec('encode', 'TransactionSignaturePayload', json.dumps(payload))
    if hashlib.sha256(base64.b64decode(encoded, validate=True)).hexdigest() != tx_hash:
        raise ValueError('receipt envelope hash differs')
    transaction = envelope['tx']['tx']
    operations = transaction['operations']
    if transaction['source_account'] != source or len(operations) != 1:
        raise ValueError('unexpected source or operation count')
    host = operations[0]['body']['invoke_host_function']['host_function']
    if host != expected_host:
        raise ValueError('receipt host function differs from built request')
    outcome = decode('TransactionResult', result['resultXdr'])
    if status == 'FAILED':
        if set(outcome['result']) != {'tx_failed'}:
            raise ValueError('FAILED status contradicts result')
        return None
    meta = decode('TransactionMeta', result['resultMetaXdr'])
    version = meta.get('v4', meta.get('v3'))
    if version is None or version['soroban_meta'] is None:
        raise ValueError('missing Soroban metadata')
    value = version['soroban_meta']['return_value']
    events = ([event for op in version['operations'] for event in op['events']]
              if 'v4' in meta else version['soroban_meta']['events'])
    preimage = codec('encode', 'InvokeHostFunctionSuccessPreImage',
                    json.dumps({'return_value': value, 'events': events}))
    digest = hashlib.sha256(base64.b64decode(preimage, validate=True)).hexdigest()
    if outcome['result'] != {'tx_success': [{'op_inner': {'invoke_host_function': {'success': digest}}}]}:
        raise ValueError('return/events do not match committed success hash')
    return plain(value)
