# Minecraft 1.16.1 endurance server

Java Edition 1.16.1の公式`server.jar`をこのディレクトリへ置き、Java 8で起動します。

```bash
java -Xms512M -Xmx1536M -jar server.jar nogui
```

初回world生成後、datapackを配置して再起動します。

```bash
mkdir -p world/datapacks/mc_ai
cp -R datapack/. world/datapacks/mc_ai/
```

サーバーconsoleで次を実行すると、Peaceful固定の49×49 block耐久コースを生成します。

```text
function mc_ai:setup
debug start
```

耐久試験終了後に`debug stop`を実行します。出力に表示されるticks/secondをTPS証跡として各Markdownレポートへ転記します。

この設定は`online-mode=false`のローカル試験専用です。各propertiesは安全な既定値として
`server-ip=127.0.0.1`へbindします。外部公開しないでください。LAN上の別端末から接続する
場合も、信頼できる閉じたnetworkであることを確認したうえで明示的にbind先を変更します。

## 自然生成地形試験

`natural-server.properties`は、固定seedの通常地形をport 25567で生成します。`natural-datapack/`をworldへ配置すると、load時にPeacefulへ設定し、180秒ごとに`spreadplayers`で全プレイヤーを中心座標 `(0, 0)` 周辺の安全な地表へ分散teleportします。これにより穴や地形へスタックしたBotを定期的に解放します。純粋な移動耐久試験にするため、Botには定期的にResistance VとWater Breathingを付与し、溺水や落下による死亡待機を防ぎます。

```bash
cp natural-server.properties server.properties
mkdir -p world/datapacks/mc_ai
cp -R natural-datapack/. world/datapacks/mc_ai/
java -Xms512M -Xmx1536M -jar server.jar nogui
```

別terminalから自然地形用runnerを実行します。既定値は10 Bots × 10分です。

```bash
MC_PORT=25567 BOT_COUNT=10 DURATION_SECS=600 \
  bash scripts/run_natural_terrain.sh
```

自然地形runnerは`WANDER=true`を指定し、方向転換、0.5秒周期のjump pulse（400ms ON / 100ms OFF）、3秒間動けなかった場合の大きな旋回を行う入力generatorを有効にします。これは経路探索ではなく、clientの動きを目視するための決定的なテスト入力です。

server teleportはclientから見ると位置補正なので、このscenarioでは補正数と距離を記録しつつ、補正0の合格条件だけを無効にします。切断0とphysics tick p99 5 ms未満は従来どおり検査します。

`natural-datapack`の定期teleportと耐久用effectは、`Natural00`〜`Natural09`を自動登録した`mc_ai_bot` tagだけに適用します。同じserverへ参加した人間プレイヤーは対象になりません。

`natural-datapack`はteleportの5秒前に`MC_AI_RESCUE_PREPARE`をsystem chatへ送ります。teleport前後のmovement producer停止を比較する切り分け試験では、runnerへ`PAUSE_ON_RESCUE_WARNING=true`を指定します。各Botは予告を受けてpacket生成を7秒間止めます。`ControlState`自体は保持され、teleportの約2秒後に自動的に移動を再開します。
