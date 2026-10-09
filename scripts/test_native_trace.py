#!/usr/bin/env python3
"""Transport-boundary regressions for original-frame validation, no game JVM."""
import socket
import tempfile
import unittest
from pathlib import Path
from run_common_native import PacketTraceProxy

class ClosedDestination:
    def sendall(self, _):
        raise BrokenPipeError(32, 'fixture peer closed')
    def shutdown(self, _):
        pass

class TraceTests(unittest.TestCase):
    def test_diagnostic_retention_never_filters_original_wire_bytes(self):
        class Destination:
            def __init__(self): self.wire = bytearray()
            def sendall(self, wire): self.wire.extend(wire)
            def shutdown(self, _): pass
        with tempfile.TemporaryDirectory() as folder:
            trace = PacketTraceProxy(1, '1.16.1', Path(folder)/'frames.jsonl',
                                     record_filter=lambda record: record['packet_id'] == 0x20)
            incoming, outgoing = socket.socketpair()
            state = {'compression': None, 'phase': 'play', 'connection': 1}
            destination = Destination()
            # One status event followed by an actual eight-byte KeepAlive.
            wire = bytes([6, 0x1b, 0, 0, 0, 42, 8, 9, 0x20]) + (71).to_bytes(8, 'big')
            try:
                outgoing.sendall(wire); outgoing.shutdown(socket.SHUT_WR)
                trace.forward(incoming, destination, 'clientbound', state)
                self.assertEqual(bytes(destination.wire), wire)
                self.assertEqual(trace.frame_count, 2)
                self.assertEqual(len(trace.frames), 1)
                self.assertEqual(trace.frames[0]['ordinal'], 2)
                self.assertEqual(trace.packet_counts[(1, 'clientbound', 'play', 0x1b)], 1)
                self.assertEqual(trace.errors, [])
            finally:
                incoming.close(); outgoing.close(); trace.close()

    def forward(self, wire, direction, requested):
        with tempfile.TemporaryDirectory() as folder:
            trace=PacketTraceProxy(1,'1.21.11',Path(folder)/'frames.jsonl')
            incoming,outgoing=socket.socketpair()
            state={'compression':None,'phase':'play','connection':1}
            with trace.lock:
                trace.connection_states.append(state)
            if requested:
                trace.expect_disconnect()
            try:
                outgoing.sendall(wire);outgoing.shutdown(socket.SHUT_WR)
                trace.forward(incoming,ClosedDestination(),direction,state)
                return list(trace.frames),list(trace.errors),list(trace.terminal_deliveries)
            finally:
                incoming.close();outgoing.close();trace.close()

    def test_complete_late_incoming_frame_retained_without_inventing_delivery(self):
        frames,errors,terminal=self.forward(bytes([2,0,17]),'clientbound',True)
        self.assertEqual(len(frames),1)
        self.assertEqual(errors,[])
        self.assertEqual(len(terminal),1)
        self.assertTrue(terminal[0]['source_frame_complete'])
        self.assertFalse(terminal[0]['delivered'])
        self.assertEqual(terminal[0]['recorded_frame_ordinal'],frames[0]['ordinal'])
        self.assertEqual(frames[0]['body_hex'],'11')

    def test_unrequested_peer_failure_and_all_outgoing_failures_remain_errors(self):
        for direction,requested in [('clientbound',False),('serverbound',False),('serverbound',True)]:
            with self.subTest(direction=direction,requested=requested):
                frames,errors,terminal=self.forward(bytes([2,0,17]),direction,requested)
                self.assertEqual(len(frames),1)
                self.assertEqual(len(errors),1)
                self.assertEqual(terminal,[])

    def test_explicit_disconnect_scopes_only_the_requested_managed_connection(self):
        with tempfile.TemporaryDirectory() as folder:
            trace=PacketTraceProxy(1,'1.21.11',Path(folder)/'frames.jsonl')
            try:
                with trace.lock:
                    trace.connection_states.extend([{'connection':1,'phase':'play'},{'connection':2,'phase':'play'}])
                trace.expect_disconnect(1)
                self.assertTrue(trace.connection_states[0]['disconnect_requested'])
                self.assertFalse(trace.connection_states[1].get('disconnect_requested',False))
                with self.assertRaises(RuntimeError):trace.expect_disconnect(99)
                self.assertEqual(len(trace.terminal_events),1)
            finally:trace.close()

    def test_partial_frames_never_become_complete_or_expected_deliveries(self):
        for direction in ['clientbound','serverbound']:
            for wire in [bytes([2]),bytes([2,0]),bytes([128])]:
                with self.subTest(direction=direction,wire=wire):
                    frames,errors,terminal=self.forward(wire,direction,True)
                    self.assertEqual(frames,[])
                    self.assertEqual(len(errors),1)
                    self.assertEqual(terminal,[])

if __name__=='__main__':
    unittest.main()
