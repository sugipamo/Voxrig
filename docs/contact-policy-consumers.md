# Contact policy consumer checks

The private Rust const tables own policy. The test-only exporter extends the
step-3 capability export with schema version, a SHA-256 policy digest, expanded
present-key axes/count, versioned classifications, lane selectors, packet
profiles, locality requirements/revocations, and evidence/scenario IDs.
The digest covers compact recursively sorted JSON excluding `policy_digest`.
Nothing reads this export at SDK runtime and it adds no public API.

```sh
python3 -B scripts/contact_policy_export.py > /tmp/contact-policy.json
python3 -B scripts/contact_policy_export.py --write-docs
python3 -B scripts/contact_policy_export.py --check
python3 -B scripts/audit_contact_capabilities.py --check
python3 -B scripts/audit_contact_packets.py --check
python3 -B scripts/audit_contact_envelopes.py --check
python3 -B scripts/check_contact_policy_consumers.py --check
```

`--write-docs` replaces only marked sections in the six contact audit/consumer
documents. CI compares freshly rendered sections with the committed text and
fails on any difference. The handwritten original-client reviews remain evidence;
exported classifications cannot establish a side-effect or numerical proof.

The consumer check compares the required Java factory/registration targets with
the fixed original-owner corpus in both scenes and with the Java factory source.
Unknown observations and exclusion probes remain in the independent corpus.
Rust admission and numerical tests retain fixed counts, exact expected values,
refusal contexts, and the earlier frozen-source/golden comparisons. The table
cannot remove an existing numerical scenario merely by changing a classification.
Original jar collection commands in the individual audits and owner runner are
preserved. Jar-independent checks use committed evidence and gz oracles; they
skip jar-dependent collection, so absent jars do not skip the policy/list checks.

<!-- contact-policy:manifest:start -->
Policy SHA-256: `e632b2900149704d48d59aa2690e80309e3f0f806532b599f45cfeac1909f672`.
Expanded present keys: 4,814,040; versioned entities: 264.

| Version | Scenario ID | Assigned types |
| --- | --- | --- |
| 1.16.1 | None | area_effect_cloud, arrow, bee, blaze, boat, cat, chest_minecart, cod, command_block_minecart, dolphin, donkey, dragon_fireball, drowned, egg, elder_guardian, end_crystal, ender_dragon, ender_pearl, enderman, endermite, evoker, evoker_fangs, experience_bottle, experience_orb, eye_of_ender, falling_block, fireball, firework_rocket, fishing_bobber, fox, furnace_minecart, ghast, guardian, hoglin, hopper_minecart, horse, husk, illusioner, iron_golem, item, item_frame, leash_knot, lightning_bolt, llama, llama_spit, magma_cube, minecart, mule, painting, panda, parrot, phantom, piglin, pillager, polar_bear, potion, pufferfish, ravager, salmon, shulker, shulker_bullet, skeleton, skeleton_horse, slime, small_fireball, snow_golem, snowball, spawner_minecart, spectral_arrow, squid, stray, strider, tnt, tnt_minecart, trader_llama, trident, tropical_fish, turtle, vex, vindicator, witch, wither, wither_skeleton, wither_skull, wolf, zoglin, zombie_horse, zombie_villager, zombified_piglin |
| 1.16.1 | Excluded | armor_stand, bat |
| 1.16.1 | Living | cow, creeper, giant, mooshroom, ocelot, pig, rabbit, sheep, silverfish, villager, wandering_trader, zombie |
| 1.16.1 | Chicken | chicken |
| 1.16.1 | RemotePlayer | player |
| 1.16.1 | SpiderEnvelope | cave_spider, spider |
| 1.21.11 | None | acacia_boat, acacia_chest_boat, allay, area_effect_cloud, armadillo, arrow, axolotl, bamboo_chest_raft, bamboo_raft, bee, birch_boat, birch_chest_boat, blaze, block_display, bogged, breeze, breeze_wind_charge, camel, camel_husk, cat, cherry_boat, cherry_chest_boat, chest_minecart, cod, command_block_minecart, copper_golem, creaking, dark_oak_boat, dark_oak_chest_boat, dolphin, donkey, dragon_fireball, drowned, egg, elder_guardian, end_crystal, ender_dragon, ender_pearl, enderman, endermite, evoker, evoker_fangs, experience_bottle, experience_orb, eye_of_ender, falling_block, fireball, firework_rocket, fishing_bobber, fox, frog, furnace_minecart, ghast, glow_item_frame, glow_squid, goat, guardian, happy_ghast, hoglin, hopper_minecart, horse, husk, illusioner, interaction, iron_golem, item, item_display, item_frame, jungle_boat, jungle_chest_boat, leash_knot, lightning_bolt, lingering_potion, llama, llama_spit, magma_cube, mangrove_boat, mangrove_chest_boat, mannequin, marker, minecart, mule, nautilus, oak_boat, oak_chest_boat, ominous_item_spawner, painting, pale_oak_boat, pale_oak_chest_boat, panda, parched, parrot, phantom, piglin, piglin_brute, pillager, polar_bear, pufferfish, ravager, salmon, shulker, shulker_bullet, skeleton, skeleton_horse, slime, small_fireball, sniffer, snow_golem, snowball, spawner_minecart, spectral_arrow, splash_potion, spruce_boat, spruce_chest_boat, squid, stray, strider, tadpole, text_display, tnt, tnt_minecart, trader_llama, trident, tropical_fish, turtle, vex, vindicator, warden, wind_charge, witch, wither, wither_skeleton, wither_skull, wolf, zoglin, zombie_horse, zombie_nautilus, zombie_villager, zombified_piglin |
| 1.21.11 | Excluded | armor_stand, bat |
| 1.21.11 | Living | cow, creeper, giant, mooshroom, ocelot, pig, rabbit, sheep, silverfish, villager, wandering_trader, zombie |
| 1.21.11 | Chicken | chicken |
| 1.21.11 | RemotePlayer | player |
| 1.21.11 | SpiderEnvelope | cave_spider, spider |

| Version | Public geometry selector | Types |
| --- | --- | --- |
| 1.16.1 | AgeablePose | cow, pig, zombie |
| 1.16.1 | PlayerPose | player |
| 1.21.11 | AgeablePose | cow, pig, zombie |
| 1.21.11 | PlayerPose | player |

| Evidence ID | Artifact |
| --- | --- |
| None | none |
| CapabilityAudit | docs/evidence/contact-capabilities-20261015.json |
| PacketAudit | data/client_api/contact_packet_audit.json |
| EnvelopeAudit | docs/evidence/contact-envelopes-20261015.json |
| OwnerOracle | data/client_api/entity_owner_oracle.json.gz |
<!-- contact-policy:manifest:end -->
