# Security policy

## Supported versions

公開release前は`main` branchのみを対象とします。最初のrelease後にsupport期間をこの文書へ
追記します。

## Reporting a vulnerability

脆弱性の疑い、悪意あるserver packetによるpanic・過大確保、認証情報の露出は公開issueへ
詳細を書かず、GitHub repositoryのPrivate vulnerability reportingから報告してください。
Repositoryでprivate reportingが利用できない場合は、再現packetやexploitを公開せず、
maintainerへ非公開の連絡経路を確認してください。

報告には影響するversionまたはcommit、再現条件、想定される影響、可能なら最小packetを含めて
ください。受領確認後、影響評価と修正方針を非公開で調整します。
