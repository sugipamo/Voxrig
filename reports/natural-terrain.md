# Endurance test result

- Status: complete
- Bots: 10
- Scenario: natural-terrain-peaceful
- Server teleports allowed: true
- Server teleport interval: 180 seconds
- Requested duration: 600 seconds
- Recorded duration: 600.000 seconds
- Movement packets: 121749
- Position corrections: 1623
- Total correction distance: 3423.131540038
- Largest correction distance: 321.777602226
- Disconnects: 0
- Worst per-Bot physics tick p99: 90µs
- Maximum physics tick: 3.191767ms
- Worst per-Bot queue lag p99: 2.073ms
- Maximum queue lag: 10.345268ms
- Process RSS: 189048 KiB

## Notes

- Minecraft Java 1.16.1、protocol 736、offline-mode、固定seedのdefault worldで実施した。
- server datapackから180秒ごとに`spreadplayers`を実行し、10 Botsを安全な地表へ分散teleportした。試験中に3回実行された。
- 自然地形での移動耐久を測るため、Resistance VとWater Breathingを定期付与して溺水・落下による死亡待機を防いだ。
- Position correctionsには意図的なteleportと、その直前に送信済みだった旧座標movement packetに対するserver応答が含まれる。そのため通常の補正0試験とは比較しない。
- 1回目と3回目のteleport直後に補正burstが発生したが収束し、その後もmovement packetは増加した。server kickとclient切断は0だった。
