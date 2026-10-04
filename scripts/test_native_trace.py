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
