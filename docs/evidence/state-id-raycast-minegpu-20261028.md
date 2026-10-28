# minegpu supplied raycast measurements (2026-10-28 JST)

These are the supplied Golemkit/minegpu prototype results, not this PR's local
microbenchmark or LIVE qualification. Prototype patch base: `502ac0f`.
The original artifacts were supplied at `~/logs/contact/fastray/`.

- `patch-voxrig-fastray.diff` SHA-256: `8bb802f84311753a728740e1a677efb432da2e9712a35f734c6ee6ddd153e78c`
- `table-final.md` SHA-256: `f7db3cdaf902aebd230db0a37d61aebc06a623121d4c96ed8f10113a40f193f0`
- `findings.md` SHA-256: `853b0dd0174079af61340e83f36a0b5c40edc1ca806e689ffdf936162a04801b`

## table-final.md

| run | success | disc | srv timeouts | plan s | CPU mean/p90 % | CPU s / wall s | observe | attack_ray | nav+aux | other failures |
|---|---|---|---|---|---|---|---|---|---|---|
| E4-32-r3 | 30/32 | 2 | 0 | 9.6 | 694/1066 | 1476/209 | 1182.5s (80%, 19.1ms/call) | 1062.8s (72.0%, n=1321697, 804us) | 4.2% | {} |
| B-32-r1 | 31/32 | 0 | 0 | 8.3 | 767/1068 | 1706/222 | 1387.9s (81%, 19.4ms/call) | 1235.7s (72.4%, n=1687817, 732us) | 3.2% | {'Diverged': 1} |
| B-32-r2 | 30/32 | 2 | 0 | 8.7 | 877/1093 | 1863/213 | 1529.2s (82%, 22.4ms/call) | 1375.9s (73.9%, n=1734596, 793us) | 3.1% | {} |
| F-32-r1 | 28/32 | 1 | 0 | 3.8 | 120/273 | 448/373 | 120.6s (27%, 1.2ms/call) | 14.8s (3.3%, n=2672500, 6us) | 14.3% | {'Stuck': 2, '時間切れ': 1} |
| F-32-r2 | 30/32 | 0 | 0 | 3.8 | 111/258 | 416/374 | 110.6s (27%, 1.2ms/call) | 14.2s (3.4%, n=2563518, 6us) | 12.8% | {'時間切れ': 1, 'Diverged': 1} |
| FF-32-r1 | 28/32 | 1 | 0 | 3.8 | 106/221 | 401/377 | 108.7s (27%, 1.2ms/call) | 13.1s (3.3%, n=2362729, 6us) | 11.1% | {'Stuck': 1, '時間切れ': 1, 'Diverged': 1} |
| FF-32-r2 | 30/32 | 0 | 0 | 3.8 | 113/241 | 422/371 | 115.3s (27%, 1.3ms/call) | 14.0s (3.3%, n=2468662, 6us) | 12.0% | {'時間切れ': 2} |
| E4-16-r1 | 16/16 | 0 | 0 | 4.4 | 378/678 | 834/220 | 0.0s (0%, 0.0ms/call) | 0.0s (0.0%, n=0, 0us) | 3.1% | {} |
| E4-16-r2 | 12/16 | 0 | 0 | 3.0 | 280/442 | 1037/370 | 0.0s (0%, 0.0ms/call) | 0.0s (0.0%, n=0, 0us) | 3.4% | {'Diverged': 2, '時間切れ': 1, 'Stuck': 1} |
| B-16-r1 | 14/16 | 0 | 0 | 3.2 | 333/503 | 1220/368 | 1045.1s (86%, 17.7ms/call) | 948.3s (77.7%, n=1606232, 590us) | 1.6% | {'時間切れ': 1, 'Stuck': 1} |
| F-16-r1 | 14/16 | 1 | 0 | 3.2 | 84/172 | 158/187 | 30.0s (19%, 1.0ms/call) | 3.0s (1.9%, n=569793, 5us) | 31.1% | {'Diverged': 1} |
| FF-16-r1 | 15/16 | 0 | 0 | 3.0 | 94/162 | 176/186 | 36.3s (21%, 1.0ms/call) | 3.3s (1.9%, n=575295, 6us) | 30.5% | {'CraftingTake': 1} |
| P-16-r1 | 16/16 | 0 | 0 | 3.9 | 916/1143 | 2276/249 | 2103.6s (92%, 68.6ms/call) | 1891.6s (83.1%, n=572576, 3304us) | 1.0% | {} |
| S-32-r1 | 23/32 | 8 | 8 | 17.7 | 675/1021 | 2405/356 | 0.0s (0%, 0.0ms/call) | 0.0s (0.0%, n=0, 0us) | 2.0% | {'Diverged': 1} |
| S-32-FF | 31/32 | 0 | 0 | 11.7 | 158/228 | 523/331 | 186.6s (36%, 1.3ms/call) | 17.8s (3.4%, n=2828375, 6us) | 8.5% | {'Stuck': 1} |

## findings.md

# raycast prototype findings (2026-10-28 JST)
- parity: unit 100000 rays 0 mismatch; live P-16-r1 ~1.9M rays (calls+random) 0 mismatch
- 32 bots: B total CPU 1706-1863s, attack_ray 72-74%, 730-790us/ray; F/FF 401-448s, attack_ray 3.3%, 6us/ray
- 16 bots: B 1220s (attack_ray 77.7%, 590us); F 158s, FF 176s (1.9%)
- S-32-FF: 31/32, disc 0, CPU 523s, vs old S-32-r1 23/32 disc 8
- spot0 swim/idle timeout: baseline E4-32-r1, E4-16-r2, B-16-r1 also -> pre-existing; F/FF-32 each 1-2
- filter (2): rays -10% only, marginal
