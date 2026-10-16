"""Offline regressions for artifact provenance and read-only mainnet checks."""
import contextlib
import io
import json
import pathlib
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import check_mainnet
import sync_wasm


COMMIT = "a" * 40
CONTROLLER = "CAUCMIN5KSXEVZ7NMXR3LZATGD5EFIEUI5XWTFLYRO2R5OTXI22WE5JX"
OLD_AGGREGATOR = "CBGUF2G2Q7HCVCWYISDXBHPVBGMYNXA7PG2VET66YBZX6IKOOV27NSMV"
NEW_AGGREGATOR = "CBMARXSYE56NPDLX42TTICM5GS5XQCYRD5VMCJME624TS6C2S53VETYU"
CFG = {"rpc_url": "https://rpc.invalid", "network_passphrase": check_mainnet.PASSPHRASE}


class ProvenanceTests(unittest.TestCase):
    def test_release_binds_every_artifact_to_one_resolved_commit(self):
        data = b"\0asm\x01\0\0\0"
        calls = []

        def run(*args):
            calls.append(args)
            if args[:2] == ("gh", "api"):
                return COMMIT
            if args[:3] == ("gh", "release", "download"):
                out = pathlib.Path(args[args.index("-D") + 1])
                manifest = {}
                for name in sync_wasm.CONTRACTS + list(sync_wasm.MOCKS):
                    (out / f"sdk-{name}.wasm").write_bytes(data)
                    manifest[name] = {"sha256": sync_wasm.sha256(data)}
                    if name in sync_wasm.CONTRACTS:
                        (out / f"{name}.wasm").write_bytes(data)
                        manifest[name].update(deploy_sha256=sync_wasm.sha256(data),
                                              code_sha256=sync_wasm.code_hash(data))
                (out / "sdk-manifest.json").write_text(json.dumps(manifest))
            return ""

        with patch.object(sync_wasm, "run", side_effect=run):
            _, _, source = sync_wasm.from_release("v1.1.0")
        self.assertEqual(source["commit"], COMMIT)
        self.assertEqual(sum(call[:2] == ("gh", "api") for call in calls), 1)
        self.assertEqual(calls[0][:2], ("gh", "api"))
        verifications = [call for call in calls if call[:3] == ("gh", "attestation", "verify")]
        self.assertEqual(len(verifications), 1 + 2 * len(sync_wasm.CONTRACTS) + len(sync_wasm.MOCKS))
        for call in verifications:
            self.assertEqual(call[call.index("--source-digest") + 1], COMMIT)
            self.assertIn("--deny-self-hosted-runners", call)

    def test_failed_attestation_stops_sync(self):
        with patch.object(sync_wasm, "run", side_effect=subprocess.CalledProcessError(1, "gh")):
            with self.assertRaises(subprocess.CalledProcessError):
                sync_wasm.verify_attestation(pathlib.Path("artifact.wasm"), COMMIT)

    def test_unresolved_commit_is_rejected(self):
        with patch.object(sync_wasm, "run", return_value="v1.1.0"):
            with self.assertRaises(SystemExit):
                sync_wasm.resolve_commit("v1.1.0")

    def test_dirty_config_is_not_recorded_as_a_clean_commit(self):
        with patch("sys.argv", ["sync_wasm.py", "--release", "v1.1.0", "--config-dir", "/tmp/source"]), \
                patch.object(sync_wasm, "from_release", return_value=({}, {}, {})), \
                patch.object(sync_wasm, "run", return_value=" M configs/networks.json"), \
                patch.object(sync_wasm, "load_config") as load:
            with self.assertRaises(SystemExit):
                sync_wasm.main()
        load.assert_not_called()

    def test_sync_pins_remote_configs_and_records_the_active_aggregator(self):
        source = {"tag": "v1.1.0", "commit": "b" * 40, "method": "release"}
        networks = {"mainnet": {**CFG, "controller": CONTROLLER, "price_aggregator": OLD_AGGREGATOR}}
        with tempfile.TemporaryDirectory() as tmp, \
                patch("sys.argv", ["sync_wasm.py", "--release", "v1.1.0"]), \
                patch.object(sync_wasm, "from_release", return_value=({}, {}, source)), \
                patch.object(sync_wasm, "resolve_commit", return_value=COMMIT), \
                patch.object(sync_wasm, "load_config", return_value={"networks": networks}) as load, \
                patch.object(sync_wasm, "live_price_aggregator", return_value=NEW_AGGREGATOR), \
                patch.object(sync_wasm, "check_deployed"), \
                patch.object(sync_wasm, "WASM_DIR", pathlib.Path(tmp) / "missing"), \
                patch.object(sync_wasm, "write_manifest") as write, \
                patch.object(sync_wasm, "write_networks"), \
                patch.object(sync_wasm.subprocess, "run"), contextlib.redirect_stdout(io.StringIO()):
            sync_wasm.main()
        load.assert_called_once_with(None, COMMIT)
        self.assertEqual(write.call_args.args[2]["config_ref"], COMMIT)
        self.assertEqual(write.call_args.args[2]["networks"]["mainnet"]["price_aggregator"], NEW_AGGREGATOR)


class MainnetTests(unittest.TestCase):
    def test_aggregator_discovery_can_never_submit_a_transaction(self):
        with patch.object(sync_wasm, "run", return_value=json.dumps(NEW_AGGREGATOR)) as run:
            self.assertEqual(sync_wasm.live_price_aggregator(CONTROLLER, CFG), NEW_AGGREGATOR)
        self.assertIn("--send=no", run.call_args.args)
        self.assertIn("--no-cache", run.call_args.args)

    def test_invalid_rpc_address_is_rejected(self):
        for value in [None, {}, "not a contract", NEW_AGGREGATOR + "extra"]:
            with self.subTest(value=value), patch.object(sync_wasm, "run", return_value=json.dumps(value)):
                with self.assertRaises(SystemExit):
                    sync_wasm.live_price_aggregator(CONTROLLER, CFG)

    def check(self, allow=False, error=None):
        manifest = {"contracts": {
            "controller": {"mainnet_contract": CONTROLLER, "deploy_sha256": "1" * 64},
            "price_aggregator": {"mainnet_contract": OLD_AGGREGATOR, "deploy_sha256": "2" * 64},
        }}
        with tempfile.TemporaryDirectory() as tmp:
            directory = pathlib.Path(tmp)
            (directory / "MANIFEST.json").write_text(json.dumps(manifest))
            args = ["check_mainnet.py"] + (["--allow-undeployed"] if allow else [])
            with patch("sys.argv", args), patch.object(check_mainnet, "WASM_DIR", directory), \
                    patch.object(check_mainnet, "live_price_aggregator", return_value=NEW_AGGREGATOR), \
                    patch.object(check_mainnet, "live_wasm_hash", side_effect=error or ["1" * 64, "3" * 64]) as fetch, \
                    contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                result = check_mainnet.main()
        self.assertEqual(fetch.call_args_list[1].args[0], NEW_AGGREGATOR)
        return result

    def test_replaced_aggregator_hash_is_checked(self):
        self.assertEqual(self.check(), 1)

    def test_pre_release_allows_hash_drift(self):
        self.assertEqual(self.check(allow=True), 0)

    def test_pre_release_cannot_hide_rpc_failure(self):
        with self.assertRaises(subprocess.CalledProcessError):
            self.check(allow=True, error=subprocess.CalledProcessError(1, "stellar"))


if __name__ == "__main__":
    unittest.main()
