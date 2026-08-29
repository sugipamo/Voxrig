# Endurance test result

- Status: complete
- Bots: 10
- Scenario: natural-prewarn-raw-metric
- Server teleports allowed: true
- Server teleport interval: 180 seconds
- Pause on rescue warning: true
- Requested duration: 240 seconds
- Recorded duration: 240.000 seconds
- Movement packets: 46207
- Server position packets: 41
- Position corrections: 31
- Total correction distance: 4.008009365
- Largest correction distance: 1.667488026
- Disconnects: 0
- Worst per-Bot physics tick p99: 142µs
- Maximum physics tick: 3.175148ms
- Worst per-Bot queue lag p99: 1.997ms
- Maximum queue lag: 18.707826ms
- Process RSS: 187428 KiB

## Notes

- 10 Botsへの意図したteleport 10 packetに対し、追加のserver position packetは31だった。
- teleport後60秒間、server position packet 41・position correction 31から増加しなかった。
- 5秒前の予告でmovement producerを停止し、teleportの約2秒後に以前の`ControlState`を自動再開した。
