# ControlRecord.degradationへの移行

Rustの`ControlRecord`構造体へ`degradation: Option<ControlDegradation>`を追加したため、
構造体literalで全fieldを列挙する外部コードには破壊的変更となる。
直接構築するテストデータには`degradation: None`を追加する。
全fieldを列挙する分解patternには`degradation`または`..`を追加する。
SDKが返すrecordの通常の読取や既存メソッドの署名は変わらない。
JSONには`degradation`（未劣化または未対応adapterではnull）が加わる。
固定schemaのconsumerはこのfieldを許可する。

1.16.1で従来の`contact_diagnostic_for(id)`だけを参照していた上位は、通常結果の
`record.degradation`を確認する。後発の破棄には`Client::control_changes_after`または
PlayerControlの同名メソッドを使い、ログ専用cursorを保存する。
`State`は保持欠落、`InvalidInput`は未来cursorであり、空履歴として処理しない。
Stateのdiagnosticを`ControlChangeGap`へdowncastすると、欠落件数と、保持suffixの
`resume_after`／明示的な再ベースライン用の`sequence`を型付きで取得できる。
Runningのまま劣化するため、停止理由`movement_refusal`へ警告を混ぜない。

1.21.11の新ログは既定の`Unsupported`。Noneを接触保証に使わない。
既存の動作・診断APIを保ち、この版への実装移植は別作業とする。
詳細は[契約と所有](control-degradation.md)を参照。
