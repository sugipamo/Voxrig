#!/usr/bin/env python3
"""Pure controller tests: in-memory control/RCON peers, no sockets, Java or cargo."""
import copy
import hashlib
import io
import json
from pathlib import Path
import signal
import subprocess
import tempfile
import unittest
from unittest.mock import Mock, patch

import run_common_native as common
import run_native_trials as runner

SPEC = json.loads(runner.SPEC.read_text())


class FakeRcon:
    def __init__(self):
        self.commands = []
        self.stream = Mock()
        self.response = "OK"

    def command(self, command):
        self.commands.append(command)
        return self.response


class ControllerTests(unittest.TestCase):
    def peer(self, trial_id="mining_api", clock=lambda: 1.0):
        trial = copy.deepcopy(next(t for t in SPEC["trials"] if t["id"] == trial_id))
        self.rcon = FakeRcon()
        self.log = io.StringIO()
        return runner.Peer(trial, 31001, self.rcon, runner.Journal(self.log), clock)

    def request(self, peer, case=None, phase="setup", elapsed=None):
        if case is None:
            case = peer.trial["cases"][peer.case_index + 1]["id"]
        return dict(schema_version=1, run_id="run", trial_id=peer.trial["id"],
                    case_id=case, request_id=peer.next_request, phase=phase,
                    game_port=peer.port, action_elapsed_ms=elapsed)

    def test_all_mining_contracts_and_inventory(self):
        for trial_id in runner.MINING:
            peer = self.peer(trial_id)
            self.assertFalse(peer.complete())
            for case in peer.trial["cases"]:
                request = self.request(peer)
                reply = peer.handle(runner.frame((json.dumps(request) + "\n").encode()))
                self.assertEqual(set(reply), set(runner.IDENTITY) | {"status"})
                self.assertEqual(reply, {**{k: request[k] for k in runner.IDENTITY}, "status": "ready"})
                self.assertFalse(any(c.startswith("op ") for c in self.rcon.commands))
                for edit in case["permitted_external_edits"]:
                    peer.handle(self.request(peer, case["id"], edit["phase"], 50))
                    self.assertEqual(self.rcon.commands[-2:], [
                        "setblock 2 -59 0 minecraft:air", "setblock 2 -59 0 minecraft:stone"])
            self.assertTrue(peer.complete())
            self.assertEqual(self.rcon.commands.count("clear NatMineBot"), len(peer.trial["cases"]))
            supplies = [c for c in self.rcon.commands if c.startswith("item replace")]
            self.assertEqual(supplies, ["item replace entity NatMineBot hotbar.0 with minecraft:cobblestone 1"] * 3
                             if trial_id == "mining_recovery" else [])
            records = [json.loads(line) for line in self.log.getvalue().splitlines()]
            self.assertEqual(records[0]["stage"], "request")
            self.assertEqual(records[-1]["stage"], "reply")

    def test_stale_duplicate_and_wrong_identity_fail_before_edits(self):
        for key, value in (("request_id", 0), ("request_id", True), ("game_port", 25572),
                           ("trial_id", "mining_raw"), ("schema_version", 2), ("run_id", ""),
                           ("case_id", "early_finish_abort"), ("action_elapsed_ms", 0)):
            peer = self.peer()
            request = self.request(peer)
            request[key] = value
            with self.assertRaises(ValueError):
                peer.handle(request)
            self.assertEqual(self.rcon.commands, [])
            with self.assertRaisesRegex(ValueError, "unusable"):
                peer.handle(self.request(peer))
        for key, value in (("request_id", 1), ("run_id", "other")):
            peer = self.peer()
            peer.handle(self.request(peer))
            request = self.request(peer)
            request[key] = value
            before = len(self.rcon.commands)
            with self.assertRaises(ValueError):
                peer.handle(request)
            self.assertEqual(len(self.rcon.commands), before)

    def external_peer(self, clock=lambda: 1.0):
        peer = self.peer(clock=clock)
        for _ in range(3):
            peer.handle(self.request(peer))
        return peer

    def test_external_deadline_is_relative_to_action_and_no_retry(self):
        for elapsed in (3000, -1, None, True, float("nan"), 4000):
            peer = self.external_peer()
            before = len(self.rcon.commands)
            with self.assertRaises(ValueError):
                peer.handle(self.request(peer, peer.trial["cases"][2]["id"], "external_edit", elapsed))
            self.assertEqual(len(self.rcon.commands), before)
        peer = self.external_peer()
        peer.clock = Mock(side_effect=[1.0, 1.2])
        with self.assertRaisesRegex(TimeoutError, "action deadline"):
            peer.handle(self.request(peer, peer.trial["cases"][2]["id"], "external_edit", 2900))
        self.assertAlmostEqual(self.rcon.stream.settimeout.call_args.args[0], 0.1)
        self.assertFalse(peer.complete())
        before = len(self.rcon.commands)
        with self.assertRaisesRegex(ValueError, "unusable"):
            peer.handle(self.request(peer))
        self.assertEqual(len(self.rcon.commands), before)

    def test_undeclared_duplicate_or_missing_edit(self):
        peer = self.peer()
        with self.assertRaises(ValueError):
            peer.handle(self.request(peer, "normal_finish", "external_edit", 50))
        peer = self.external_peer()
        with self.assertRaisesRegex(ValueError, "missing external edit"):
            peer.handle(self.request(peer))
        peer = self.external_peer()
        request = self.request(peer, peer.trial["cases"][2]["id"], "external_edit", 50)
        peer.handle(request)
        request["request_id"] += 1
        with self.assertRaisesRegex(ValueError, "duplicate"):
            peer.handle(request)

    def test_rcon_rejection_preserves_partial_journal(self):
        peer = self.peer()
        self.rcon.response = "Unknown or incomplete command"
        with self.assertRaisesRegex(RuntimeError, "rejected"):
            peer.handle(self.request(peer))
        self.assertFalse(peer.usable)
        self.assertIn("Unknown or incomplete", self.log.getvalue())
        self.assertNotIn('"stage": "reply"', self.log.getvalue())

    def test_control_framing(self):
        for line in (b"{}", b"{}\n", b"[]\n", b"x" * 4096 + b"\n", b"invalid\n"):
            with self.assertRaises((ValueError, json.JSONDecodeError)):
                runner.frame(line)

    def test_fake_control_transport_fragmentation_and_close(self):
        peer = self.peer("mining_raw")
        request = (json.dumps(self.request(peer)) + "\n").encode()
        stream = Mock()
        stream.recv.side_effect = [request[:10], request[10:], b""]
        listener = Mock()
        listener.accept.return_value = (stream, ("127.0.0.1", 1))
        process = Mock()
        process.poll.return_value = None
        process.wait.return_value = 1
        self.assertEqual(runner.control_loop(listener, process, peer, 2), 1)
        self.assertEqual(json.loads(stream.sendall.call_args.args[0])["status"], "ready")
        stream.close.assert_called_once()
        self.assertFalse(peer.complete())

    def test_rcon_timeout_propagates_and_closes_control(self):
        peer = self.peer()
        self.rcon.command = Mock(side_effect=TimeoutError("RCON timeout"))
        stream = Mock()
        stream.recv.return_value = (json.dumps(self.request(peer)) + "\n").encode()
        listener = Mock()
        listener.accept.return_value = (stream, None)
        process = Mock()
        process.poll.return_value = None
        with self.assertRaisesRegex(TimeoutError, "RCON timeout"):
            runner.control_loop(listener, process, peer, 2)
        stream.close.assert_called_once()
        stream.sendall.assert_not_called()

    def test_process_group_cleanup_escalates(self):
        process = Mock(pid=123)
        process.wait.side_effect = [subprocess.TimeoutExpired("cargo", 10), 0, 0]
        with patch.object(runner.os, "killpg") as kill:
            runner.stop_group(process)
        self.assertEqual(kill.call_args_list[0].args, (123, signal.SIGTERM))
        self.assertEqual(kill.call_args_list[1].args, (123, signal.SIGKILL))

    def test_failed_bootstrap_retains_report_and_stops_server(self):
        with tempfile.TemporaryDirectory() as directory:
            folder = Path(directory) / "trial"
            server = Mock(pid=123, returncode=0)
            server.poll.return_value = None
            listener = Mock()
            listener.__enter__ = Mock(return_value=listener)
            listener.__exit__ = Mock(return_value=False)
            listener.getsockname.return_value = ("127.0.0.1", 31003)
            with patch.object(runner, "configure"), patch.object(runner, "free_port", side_effect=[31001, 31002]), \
                    patch.object(runner.socket, "socket", return_value=listener), \
                    patch.object(runner.subprocess, "Popen", return_value=server), \
                    patch.object(runner, "until", side_effect=TimeoutError("bootstrap")), \
                    patch.object(runner, "stop_group") as stop:
                with self.assertRaisesRegex(TimeoutError, "bootstrap"):
                    runner.run_trial(self.peer().trial, Path("jar"), folder, True)
            stop.assert_any_call(server)
            server.stdin.write.assert_called_with("stop\n")
            report = json.loads((folder / "report.json").read_text())
            self.assertEqual(report["result"], "FAIL")
            self.assertEqual(report["stage"], "stopped")
            self.assertIn("bootstrap", report["error"])
            self.assertTrue((folder / "server.log").exists())

    def test_trial_panic_and_timeout_keep_partial_output_and_reap_both_groups(self):
        for failure in ("panic", "timeout", "interrupt"):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory:
                folder = Path(directory) / "trial"
                server = Mock(pid=123, returncode=0)
                server.poll.return_value = None
                cargo = Mock(pid=456, returncode=101)
                listener = Mock()
                listener.__enter__ = Mock(return_value=listener)
                listener.__exit__ = Mock(return_value=False)
                listener.getsockname.return_value = ("127.0.0.1", 31003)
                rcon = FakeRcon()
                rcon.close = Mock()

                def fail(*_args):
                    (folder / "trial.json").write_text('{"partial":true}')
                    with (folder / "test.log").open("a") as log:
                        log.write("test result: FAILED. 0 passed; 1 failed\n")
                    if failure == "timeout":
                        raise TimeoutError("trial deadline")
                    if failure == "interrupt":
                        raise KeyboardInterrupt("signal")
                    return 101

                with patch.object(runner, "configure"), \
                        patch.object(runner, "free_port", side_effect=[31001, 31002]), \
                        patch.object(runner.socket, "socket", return_value=listener), \
                        patch.object(runner.subprocess, "Popen", side_effect=[server, cargo]), \
                        patch.object(runner, "until", side_effect=[rcon, True]), \
                        patch.object(runner, "control_loop", side_effect=fail), \
                        patch.object(runner, "stop_group") as stop:
                    with self.assertRaises((RuntimeError, TimeoutError, KeyboardInterrupt)):
                        runner.run_trial(self.peer().trial, Path("jar"), folder, True)
                stop.assert_any_call(server)
                stop.assert_any_call(cargo)
                rcon.close.assert_called_once()
                report = json.loads((folder / "report.json").read_text())
                self.assertEqual(report["result"], "FAIL")
                self.assertEqual(report["stage"], "stopped")
                self.assertTrue(json.loads((folder / "trial.json").read_text())["partial"])
                self.assertIn("FAILED", (folder / "test.log").read_text())

    def test_silent_control_deadline(self):
        peer = self.peer()
        listener, process = Mock(), Mock()
        with patch.object(runner.time, "monotonic", side_effect=[0, 3]):
            with self.assertRaisesRegex(TimeoutError, "process deadline"):
                runner.control_loop(listener, process, peer, 2)
        listener.accept.assert_not_called()

    def test_no_eula_write_without_opt_in(self):
        with tempfile.TemporaryDirectory() as directory:
            folder = Path(directory)
            with self.assertRaisesRegex(ValueError, "accept-eula"):
                runner.configure(folder, 1, 2, "password", False)
            self.assertEqual(list(folder.iterdir()), [])

    def test_metadata_hash_mismatch_rejects_supplied_jar(self):
        manifest = json.dumps({"versions": [{"id": "1.21.11", "url": "https://example.invalid/meta",
                                             "sha1": "0" * 40}]}).encode()
        with tempfile.TemporaryDirectory() as directory:
            folder = Path(directory)
            jar = folder / "server.jar"
            jar.write_bytes(b"jar")
            with patch.object(common.urllib.request, "urlopen", side_effect=[io.BytesIO(manifest), io.BytesIO(b"wrong")]):
                with self.assertRaisesRegex(RuntimeError, "metadata SHA-1 mismatch"):
                    common.download("1.21.11", downloads=folder / "downloads", server_jar=jar)
            self.assertEqual(jar.read_bytes(), b"jar")

    def test_supplied_jar_verified_against_fresh_metadata_without_download(self):
        metadata = json.dumps({"downloads": {"server": {
            "url": "https://example.invalid/server", "sha1": hashlib.sha1(b"jar").hexdigest()}}}).encode()
        manifest = json.dumps({"versions": [{"id": "1.21.11", "url": "https://example.invalid/meta",
                                             "sha1": hashlib.sha1(metadata).hexdigest()}]}).encode()
        with tempfile.TemporaryDirectory() as directory:
            folder = Path(directory)
            jar = folder / "server.jar"
            jar.write_bytes(b"jar")
            with patch.object(common.urllib.request, "urlopen", side_effect=[io.BytesIO(manifest), io.BytesIO(metadata)]) as fetch:
                actual, _ = common.download("1.21.11", downloads=folder / "downloads", server_jar=jar)
                self.assertEqual(actual, jar)
                self.assertEqual(fetch.call_count, 2)
            jar.write_bytes(b"wrong")
            with self.assertRaisesRegex(RuntimeError, "server SHA-1 mismatch"):
                common.download("1.21.11", downloads=folder / "downloads", server_jar=jar)
            self.assertEqual(jar.read_bytes(), b"wrong")


if __name__ == "__main__":
    unittest.main()
