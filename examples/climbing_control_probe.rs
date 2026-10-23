//! JSON-line driver for `scripts/run_climbing_control.py`; common API only.
use std::io::Write;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use voxrig::client::control::Controls;
use voxrig::client::prelude::*;

fn interpolation_pose(value: &serde_json::Value) -> anyhow::Result<LivingInterpolationPose> {
    let mut pose: LivingInterpolationPose = serde_json::from_value(value.clone())?;
    if let (Some(position), Some(rotation)) = (
        value["position_bits"].as_array(),
        value["rotation_bits"].as_array(),
    ) {
        anyhow::ensure!(
            position.len() == 3 && rotation.len() == 2,
            "invalid original pose bits"
        );
        for (i, bits) in position.iter().enumerate() {
            pose.position[i] = f64::from_bits(u64::from_str_radix(bits.as_str().unwrap(), 16)?);
        }
        for (i, bits) in rotation.iter().enumerate() {
            pose.rotation[i] = f32::from_bits(bits.as_u64().unwrap().try_into()?);
        }
    }
    Ok(pose)
}

fn contact_body(value: &serde_json::Value) -> anyhow::Result<LivingContactBody> {
    let mut body: LivingContactBody = serde_json::from_value(value.clone())?;
    for (key, target) in [
        ("position_bits", &mut body.push.position),
        ("velocity_bits", &mut body.push.velocity),
    ] {
        if let Some(bits) = value["push"][key].as_array() {
            anyhow::ensure!(bits.len() == 3, "invalid original vector bits");
            for (i, b) in bits.iter().enumerate() {
                target[i] = f64::from_bits(u64::from_str_radix(b.as_str().unwrap(), 16)?);
            }
        }
    }
    if let Some(bits) = value["bounds_bits"].as_array() {
        anyhow::ensure!(bits.len() == 6, "invalid original box bits");
        let f = |i: usize| -> anyhow::Result<f64> {
            Ok(f64::from_bits(u64::from_str_radix(
                bits[i].as_str().unwrap(),
                16,
            )?))
        };
        body.bounds = voxrig::client::Aabb {
            min_x: f(0)?,
            min_y: f(1)?,
            min_z: f(2)?,
            max_x: f(3)?,
            max_y: f(4)?,
            max_z: f(5)?,
        };
    }
    Ok(body)
}

fn indexed_body(value: &serde_json::Value) -> anyhow::Result<LivingContactIndexedBody> {
    let mut body: LivingContactIndexedBody = serde_json::from_value(value.clone())?;
    body.body = contact_body(&value["body"])?;
    Ok(body)
}

fn packet_diagnostics(model: &LivingPacketModel) -> serde_json::Value {
    let frame = model.frame();
    let pose = frame.interpolation.pose;
    serde_json::json!({"frame":frame,"pose_bits":{"position":pose.position.map(|n|format!("{:x}",n.to_bits())),"rotation":pose.rotation.map(f32::to_bits)}})
}

fn replay_diagnostics(model: &LivingEntityPacketReplay) -> serde_json::Value {
    let frame = model.frame();
    let pose = frame.model.interpolation.pose;
    serde_json::json!({"frame":frame,"pose_bits":{"position":pose.position.map(|n|format!("{:x}",n.to_bits())),"rotation":pose.rotation.map(f32::to_bits)}})
}

fn lighting_samples(
    chunks: &[voxrig::client::ChunkObservation],
    positions: &[[i32; 3]],
) -> serde_json::Value {
    serde_json::json!(
        positions
            .iter()
            .map(|&position| {
                let chunk = chunks.iter().find(|c| {
                    c.position == [position[0].div_euclid(16), position[2].div_euclid(16)]
                });
                serde_json::json!({"position":position,
            "state":chunk.and_then(|c|c.block(position)),
            "sky_light":chunk.and_then(|c|c.sky_light(position)),
            "block_light":chunk.and_then(|c|c.block_light(position)),
            "receive_sequence":chunk.map(|c|c.receive_sequence),
            "session":chunk.map(|c|c.session)})
            })
            .collect::<Vec<_>>()
    )
}

fn face(request: &serde_json::Value) -> anyhow::Result<voxrig::client::BlockFace> {
    use voxrig::client::BlockFace;
    Ok(match request["face"].as_str() {
        Some("Down") => BlockFace::Down,
        Some("Up") => BlockFace::Up,
        Some("North") => BlockFace::North,
        Some("South") => BlockFace::South,
        Some("West") => BlockFace::West,
        Some("East") => BlockFace::East,
        _ => anyhow::bail!("invalid probe face"),
    })
}

fn map_pixels(context: Option<&MapObservation>) -> serde_json::Value {
    serde_json::json!({"context":context,"pixels":context.map(|map| {
        (0..128u8).flat_map(|y| (0..128u8).map(move |x| {
            map.pixel(x,y).map(|pixel| match pixel.source {
                ValueSource::Received { sequence } => [u64::from(pixel.value),sequence],
                _ => panic!("map pixel was not an original receipt"),
            })
        })).collect::<Vec<_>>()
    })})
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port = std::env::var("VOXRIG_PORT")?.parse()?;
    let client = Client::connect(ConnectionConfig::offline_from_env(
        Server::new("127.0.0.1", port),
        std::env::var("VOXRIG_PROBE_USERNAME").unwrap_or_else(|_| "ClimbingProbe".into()),
    )?)
    .await?;
    client.wait_until_ready().await?;
    println!("{}", serde_json::json!({"ready":true}));
    std::io::stdout().flush()?;
    let survival = client.survival();
    let mut saved_interaction_target = None;
    let mut mounted = None;
    let mut saved_chunk = None;
    let mut saved_map = None;
    let mut saved_lighting = Vec::new();
    let mut interpolation: Option<LivingInterpolation> = None;
    let mut contact_index: Option<LivingContactIndex> = None;
    let mut entity_history_cursor = None;
    let mut packet_models = std::collections::BTreeMap::<u64, LivingPacketModel>::new();
    let mut tick_order: Option<LivingEntityTickOrder> = None;
    let mut packet_replay: Option<LivingEntityPacketReplay> = None;
    let mut packet_replay_cursor = None;
    let mut packet_replay_page = None;
    let creative_vehicle = std::env::var("VOXRIG_VEHICLE_MODE").as_deref() == Ok("creative");
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let request: serde_json::Value = serde_json::from_str(&line)?;
        let value = match request["command"].as_str().unwrap_or_default() {
            "packet_model_new"=>{
                let key=request["key"].as_u64().unwrap();
                anyhow::ensure!(packet_models.len()<2*MAX_LIVING_TICK_BODIES||packet_models.contains_key(&key),"model driver budget exceeded");
                let result=LivingPacketModel::new(client.version(),interpolation_pose(&request["pose"])?,request["ordinal"].as_u64().unwrap_or(0));
                let status=result.as_ref().map(|_|());if let Ok(model)=&result {packet_models.insert(key,model.clone());}
                serde_json::to_value(status)?
            },
            "packet_model_apply"|"packet_model_step"|"packet_model_cancel"|"packet_model_pose"=>{
                let model=packet_models.get_mut(&request["key"].as_u64().unwrap()).unwrap();
                let result=match request["command"].as_str().unwrap(){
                    "packet_model_apply"=>model.apply(request["ordinal"].as_u64().unwrap(),serde_json::from_value(request["packet"].clone())?,serde_json::from_value(request["guards"].clone())?),
                    "packet_model_step"=>model.step(request["collision_free"].as_bool()).map(|_|true),
                    "packet_model_pose"=>model.set_model_pose(interpolation_pose(&request["pose"])?).map(|_|true),
                    _=>model.cancel().map(|_|true),
                };
                serde_json::json!({"result":result,"model":packet_diagnostics(model)})
            },
            "packet_model_frame"=>packet_diagnostics(&packet_models[&request["key"].as_u64().unwrap()]),
            "tick_order_new"=>{tick_order=Some(LivingEntityTickOrder::new(client.version()));packet_models.clear();serde_json::json!({"created":true})},
            "tick_order_admit"=>serde_json::to_value(tick_order.as_mut().unwrap().admit(serde_json::from_value(request["body"].clone())?))?,
            "tick_order_update"=>serde_json::to_value(tick_order.as_mut().unwrap().update(serde_json::from_value(request["body"].clone())?))?,
            "tick_order_remove"=>serde_json::to_value(tick_order.as_mut().unwrap().remove_membership(request["native_id"].as_i64().unwrap().try_into()?))?,
            "tick_order_begin"=>serde_json::to_value(tick_order.as_mut().unwrap().begin_tick(request["frozen"].as_bool().unwrap()))?,
            "tick_order_next"=>serde_json::to_value(tick_order.as_mut().unwrap().next_tick())?,
            "tick_order_finish"=>serde_json::to_value(tick_order.as_mut().unwrap().finish_tick())?,
            "tick_order_invalidate"=>{tick_order.as_mut().unwrap().invalidate();serde_json::json!({"invalidated":true})},
            "packet_replay_new"=>{
                packet_replay=None;packet_replay_cursor=None;packet_replay_page=None;
                let current=client.entities().await?;
                let name=request["type_name"].as_str().unwrap_or("minecraft:cow");
                let target=current.entities.iter().filter(|e|e.motion.entity.type_name.as_deref()==Some(name)).max_by_key(|e|e.motion.entity.id.spawn_sequence()).map(|e|e.motion.entity.id);
                let mut cursor=None;let mut latest_spawn=None;let mut pages=Vec::new();let mut frontier=None;
                for _ in 0..9 {
                    let page=client.entity_history_after(cursor,1024).await?;
                    anyhow::ensure!(page.gap.is_none(),"spawn/history baseline was evicted");
                    frontier.get_or_insert(page.latest_cursor.ordinal());
                    for record in &page.records {if matches!(&record.kind,EntityHistoryKind::Spawn(m) if Some(m.entity.id)==target){latest_spawn=Some(record.clone());}}
                    cursor=Some(page.next_cursor);let done=page.next_cursor.ordinal()>=frontier.unwrap();pages.push(page);if done {break}
                }
                let result=if let Some(spawn)=latest_spawn {LivingEntityPacketReplay::from_spawn(&spawn)}else{Err(LivingReplayUnavailable::MissingSpawn)};
                let mut application=Ok(0);
                if let Ok(mut replay)=result.clone(){
                    let guards=serde_json::from_value(request["guards"].clone())?;
                    for page in &pages {match replay.apply_page(page,guards){Ok(n)=>*application.as_mut().unwrap()+=n,Err(e)=>{application=Err(e);break}}}
                    packet_replay=Some(replay);packet_replay_cursor=cursor;packet_replay_page=pages.last().cloned();
                }
                serde_json::json!({"created":result.map(|_|()),"applied":application,"model":packet_replay.as_ref().map(replay_diagnostics),"history":pages})
            },
            "packet_replay_history"|"packet_replay_duplicate"=>{
                let page=if request["command"]=="packet_replay_duplicate" {packet_replay_page.clone().unwrap()}else{client.entity_history_after(packet_replay_cursor,1024).await?};
                let replay=packet_replay.as_mut().unwrap();let result=replay.apply_page(&page,serde_json::from_value(request["guards"].clone())?);
                if result.is_ok(){packet_replay_cursor=Some(page.next_cursor);packet_replay_page=Some(page.clone());}
                serde_json::json!({"applied":result,"model":replay_diagnostics(replay),"history":page})
            },
            "packet_replay_step"|"packet_replay_cancel"=>{
                let replay=packet_replay.as_mut().unwrap();let result=if request["command"]=="packet_replay_step" {replay.step(request["collision_free"].as_bool())}else{replay.cancel()};
                serde_json::json!({"result":result.map(|_|()),"model":replay_diagnostics(replay)})
            },
            "packet_replay_invalidate"=>{packet_replay.as_mut().unwrap().invalidate();serde_json::json!({"invalidated":true,"model":packet_replay.as_ref().map(replay_diagnostics)})},
            "entity_history" => {
                if request["reset"] == true { entity_history_cursor = None; }
                let maximum = request["maximum"].as_u64().unwrap_or(1024).try_into()?;
                let history = client.entity_history_after(entity_history_cursor, maximum).await?;
                entity_history_cursor = Some(if request["tail"] == true { history.latest_cursor } else { history.next_cursor });
                serde_json::to_value(history)?
            },
            "revoke_entity_history" => {
                if let Some(replay)=packet_replay.as_mut(){replay.invalidate();}
                let revoked = client.revoke_connection();
                let history = client.entity_history_after(None, 1024).await?;
                println!("{}", serde_json::json!({"revoked":revoked,"history":history,"replay":packet_replay.as_ref().map(replay_diagnostics)}));
                std::io::stdout().flush()?;
                return Ok(());
            },
            "interpolation_new" => {
                let pose = interpolation_pose(&request["pose"])?;
                interpolation = Some(LivingInterpolation::new(client.version(), pose, request["length"].as_u64().unwrap().try_into()?).map_err(|e| anyhow::anyhow!("{e:?}"))?);
                serde_json::to_value(interpolation.as_ref().unwrap().frame())?
            },
            "interpolation_target" | "interpolation_step" | "interpolation_cancel" => {
                let model = interpolation.as_mut().unwrap();
                let result = match request["command"].as_str().unwrap() {
                    "interpolation_target" => model.set_target(if request["before"].is_object() { interpolation_pose(&request["before"])? } else { model.frame().pose }, interpolation_pose(&request["pose"])?),
                    "interpolation_step" => model.step(interpolation_pose(&request["pose"])?, request["collision_free"].as_bool()),
                    _ => Ok(model.cancel()),
                };
                let bits = result.as_ref().ok().map(|f| serde_json::json!({
                    "position":f.pose.position.map(|n| format!("{:x}", n.to_bits())),
                    "rotation":f.pose.rotation.map(f32::to_bits)
                }));
                serde_json::json!({"model":result,"pose_bits":bits,"retained":model.frame()})
            },
            "contact_index_new" => {
                contact_index = Some(LivingContactIndex::new(client.version()));
                serde_json::json!({"created":true})
            },
            "contact_index_chunk" => serde_json::to_value(contact_index.as_mut().unwrap().set_chunk_known(
                serde_json::from_value(request["chunk"].clone())?, request["known"].as_bool().unwrap()
            ))?,
            "contact_index_upsert" => serde_json::to_value(contact_index.as_mut().unwrap().upsert(indexed_body(&request["entity"])?))?,
            "contact_index_remove" => serde_json::to_value(contact_index.as_mut().unwrap().remove(request["key"].as_u64().unwrap()))?,
            "contact_index_visibility" => serde_json::to_value(contact_index.as_mut().unwrap().set_section_accessible(
                serde_json::from_value(request["section"].clone())?,request["accessible"].as_bool().unwrap()
            ))?,
            "contact_index_query" => serde_json::to_value(contact_index.as_ref().unwrap().query(request["client_side"].as_bool().unwrap(),&indexed_body(&request["querying"])?))?,
            "contact_index_invalidate" => {
                contact_index.as_mut().unwrap().invalidate();
                serde_json::json!({"invalidated":true})
            },
            "teams" => serde_json::to_value(client.teams().await?)?,
            "living_contacts" => {
                let querying = contact_body(&request["querying"])?;
                let candidates: Vec<LivingContactCandidate> = request["candidates"].as_array().unwrap().iter().map(|v| -> anyhow::Result<_> {
                    let mut c: LivingContactCandidate = serde_json::from_value(v.clone())?;
                    c.body = contact_body(&v["body"])?;
                    Ok(c)
                }).collect::<anyhow::Result<_>>()?;
                let model = model_living_contacts(client.version(), request["client_side"].as_bool().unwrap_or(true), &querying, &candidates);
                let velocity_bits = model.as_ref().ok().map(|m| {
                    serde_json::json!({"querying":m.querying_velocity.map(|n|format!("{:x}",n.to_bits())),
                        "candidates":m.candidates.iter().map(|c|c.push.as_ref().map(|p|p.first_velocity.map(|n|format!("{:x}",n.to_bits())))).collect::<Vec<_>>()})
                });
                serde_json::json!({"model":model,"velocity_bits":velocity_bits})
            }
            "living_pair_push" => {
                let input: LivingPairPushInput = serde_json::from_value(request["input"].clone())?;
                let model = model_living_pair_push(client.version(), &input);
                let bits = model.as_ref().ok().map(|m| {
                    [m.first_velocity, m.second_velocity]
                        .map(|v| v.map(|n| format!("{:x}", n.to_bits())))
                });
                serde_json::json!({"model":model,"velocity_bits":bits})
            },
            "entity_geometry" => serde_json::json!(client.entities().await?.entities.iter()
                .map(|entity| serde_json::json!({"entity":entity.motion.entity,"geometry":entity.derived_geometry(),"scale":entity.scale(),"metadata_complete":entity.metadata_complete}))
                .collect::<Vec<_>>()),
            "entity_poses" => serde_json::json!(client.entities().await?.entities.iter()
                .filter(|entity| entity.motion.entity.type_name.as_deref() == Some("minecraft:player"))
                .map(|entity| serde_json::json!({"entity":entity.motion.entity,"pose":entity.pose()}))
                .collect::<Vec<_>>()),
            "entity_sleeping" => serde_json::json!(client.entities().await?.entities.iter()
                .map(|entity| serde_json::json!({"entity":entity.motion.entity,"sleeping":entity.sleeping(),"pose":entity.pose()}))
                .collect::<Vec<_>>()),
            "use_item" => serde_json::to_value(survival.use_item(Hand::Main).await?)?,
            "map_context" => {
                let context = client
                    .map_observation(request["id"].as_i64().unwrap().try_into()?)
                    .await?;
                if request["save"] == true {
                    saved_map = context.clone();
                }
                map_pixels(context.as_ref())
            }
            "saved_map_context" => map_pixels(saved_map.as_ref()),
            "map_context_disconnect" => {
                client.disconnect().await?;
                let context = client
                    .map_observation(request["id"].as_i64().unwrap().try_into()?)
                    .await?;
                let mut response = map_pixels(context.as_ref());
                response["saved"] = map_pixels(saved_map.as_ref());
                println!("{response}");
                std::io::stdout().flush()?;
                return Ok(());
            }
            "prepare" => {
                let target: [f64; 3] = serde_json::from_value(request["position"].clone())?;
                tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        let state = client.player_state().await?;
                        if state.received_pose.as_ref().is_some_and(|p| {
                            p.position
                                .iter()
                                .zip(target)
                                .all(|(a, b)| (a - b).abs() < 0.01)
                        }) {
                            break Ok::<_, anyhow::Error>(());
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                })
                .await??;
                client
                    .wait_for_loaded(
                        Region {
                            min: [-4, 60, -4],
                            max: [4, 84, 4],
                        },
                        Duration::from_secs(10),
                    )
                    .await?;
                // Chunk receipt precedes subsequent block/entity updates.
                tokio::time::sleep(Duration::from_millis(200)).await;
                serde_json::to_value(client.player_state().await?)?
            }
            "respawn" => serde_json::to_value(client.respawn().await?)?,
            "player" => serde_json::to_value(client.player_state().await?)?,
            "explosions" => serde_json::to_value(client.explosions_after(request["after"].as_u64().unwrap_or(0)).await?)?,
            "lighting" => {
                let positions: Vec<[i32; 3]> =
                    serde_json::from_value(request["positions"].clone())?;
                let columns: std::collections::BTreeSet<_> = positions
                    .iter()
                    .map(|p| [p[0].div_euclid(16), p[2].div_euclid(16)])
                    .collect();
                let mut chunks = Vec::new();
                for column in columns {
                    if let Some(chunk) = client.chunk(column).await? {
                        chunks.push(chunk);
                    }
                }
                let value = lighting_samples(&chunks, &positions);
                if request["save"] == true {
                    saved_lighting = chunks;
                }
                value
            }
            "saved_lighting" => lighting_samples(
                &saved_lighting,
                &serde_json::from_value::<Vec<[i32; 3]>>(request["positions"].clone())?,
            ),
            "entity_interact_at" => {
                use voxrig::client::Hand;
                let target = if request["saved"] == true {
                    saved_interaction_target.ok_or_else(|| anyhow::anyhow!("no saved interaction target"))?
                } else {
                    let entities = client.entity_spawns().await?;
                    let target = entities.entities.iter().find(|e| e.type_name.as_deref() == request["type"].as_str())
                        .ok_or_else(|| anyhow::anyhow!("interaction fixture missing"))?.id;
                    saved_interaction_target = Some(target);
                    target
                };
                let hit = serde_json::from_value(request["hit"].clone())?;
                let hand = if request["offhand"] == true { Hand::Off } else { Hand::Main };
                let result = if request["mode"] == "creative" {
                    client.creative().interact_entity_at(target, hit, hand, false).await
                } else {
                    survival.interact_entity_at(target, hit, hand, false).await
                };
                match result {
                    Ok(receipt) => serde_json::json!({"target": target, "dispatch": receipt}),
                    Err(error) => serde_json::json!({"target": target, "error": error.to_string(), "kind": format!("{:?}", error.kind())}),
                }
            }
            "entity_target" => {
                let result = if request["mode"] == "creative" {
                    client.creative().target_entities().await
                } else {
                    client.survival().target_entities().await
                };
                match result {
                    Ok(value) => serde_json::json!({"observation": value}),
                    Err(error) => serde_json::json!({"error": error.to_string(), "kind": format!("{:?}", error.kind())}),
                }
            }
            "contextual_target" => {
                let result = if request["mode"] == "creative" {
                    client.creative().target_block_with_context().await
                } else {
                    client.survival().target_block_with_context().await
                };
                match result {
                    Ok(value) => serde_json::json!({"observation": value}),
                    Err(error) => serde_json::json!({"error": error.to_string(), "kind": format!("{:?}", error.kind())}),
                }
            }
            "place_slab" => {
                let clicked = serde_json::from_value(request["clicked"].clone())?;
                let target = serde_json::from_value(request["target"].clone())?;
                let before = serde_json::from_value(request["before"].clone())?;
                let result = survival.place_slab(clicked, face(&request)?, target, before).await;
                match result {
                    Ok(record) => serde_json::json!({"record": record}),
                    Err(error) => serde_json::json!({"error": error.to_string(), "kind": format!("{:?}", error.kind())}),
                }
            }
            "slab_placement_record" => serde_json::to_value(survival.slab_placement_record().await?)?,
            "place_cube_replacing" => {
                let clicked = serde_json::from_value(request["clicked"].clone())?;
                let target = serde_json::from_value(request["target"].clone())?;
                let before = serde_json::from_value(request["before"].clone())?;
                let result = survival.place_cube_replacing(clicked, face(&request)?, target, before).await;
                match result {
                    Ok(record) => serde_json::json!({"record": record}),
                    Err(error) => serde_json::json!({"error": error.to_string(), "kind": format!("{:?}", error.kind())}),
                }
            }
            "replaceable_cube_placement_record" => serde_json::to_value(survival.replaceable_cube_placement_record().await?)?,
            "place_cube" => {
                let result = survival.place_cube(serde_json::from_value(request["clicked"].clone())?, face(&request)?).await;
                match result {
                    Ok(record) => serde_json::json!({"record": record}),
                    Err(error) => serde_json::json!({"error": error.to_string(), "kind": format!("{:?}", error.kind())}),
                }
            }
            "placement_check" => serde_json::to_value(
                survival
                    .placement_check(
                        serde_json::from_value(request["support"].clone())?,
                        face(&request)?,
                    )
                    .await?,
            )?,
            "use_on_block" => {
                let target = serde_json::from_value(request["support"].clone())?;
                let face = face(&request)?;
                let cursor = serde_json::from_value(request["cursor"].clone())?;
                let result = if client.player_state().await?.game_mode == Some(GameMode::Creative) {
                    client.creative().use_on_block(target, face, cursor, Hand::Main).await
                } else { survival.use_on_block(target, face, cursor, Hand::Main).await };
                if request["expect_rejected"] == true {
                    let error = result.expect_err("out-of-range hit unexpectedly admitted");
                    serde_json::json!({"rejected":true,"kind":format!("{:?}",error.kind()),"message":error.to_string()})
                } else {
                    serde_json::to_value(result?)?
                }
            }
            "select_hotbar" => {
                survival.select_hotbar(request["slot"].as_u64().unwrap().try_into()?).await?;
                serde_json::to_value(client.player_state().await?)?
            }
            "drop_selected" => {
                use voxrig::client::inventory::SelectedDropAmount;
                let amount = match request["amount"].as_str() {
                    Some("one") => SelectedDropAmount::One,
                    Some("all") => SelectedDropAmount::All,
                    _ => anyhow::bail!("drop amount must be one or all"),
                };
                let record = if client.player_state().await?.game_mode == Some(GameMode::Creative) {
                    client.creative().drop_selected(amount).await?
                } else {
                    survival.drop_selected(amount).await?
                };
                serde_json::to_value(record)?
            }
            "selected_drop_record" => serde_json::to_value(survival.selected_drop_record().await?)?,
            "swap_hands" => {
                let record = if client.player_state().await?.game_mode == Some(GameMode::Creative) {
                    client.creative().swap_hands().await?
                } else {
                    survival.swap_hands().await?
                };
                serde_json::to_value(record)?
            }
            "hand_swap_record" => serde_json::to_value(survival.hand_swap_record().await?)?,
            "inventory_move" => {
                use std::future::Future;
                use std::task::Poll;
                use voxrig::client::inventory::{InventoryClickStage, InventorySource};
                let from = request["from"].as_u64().unwrap().try_into()?;
                let to = request["to"].as_u64().unwrap().try_into()?;
                let mut parent = InventoryMoveRecord::new(InventorySource::Player, from, to);
                let creative = client.player_state().await?.game_mode == Some(GameMode::Creative);
                let result = if request["cancel_after_first"] == true {
                    let previous = survival.inventory_click_record().await?.map(|r| r.id);
                    {
                        let mut helper = std::pin::pin!(survival.move_inventory(&mut parent));
                        tokio::time::timeout(Duration::from_secs(5), async {
                            loop {
                                if let Some(child) = survival.inventory_click_record().await? {
                                    if Some(child.id) != previous && child.stage == InventoryClickStage::ObservedClicked {
                                        break Ok::<_, anyhow::Error>(());
                                    }
                                }
                                if let Poll::Ready(result) = std::future::poll_fn(|cx| Poll::Ready(helper.as_mut().poll(cx))).await {
                                    anyhow::bail!("move completed before cancellation: {result:?}");
                                }
                                tokio::time::sleep(Duration::from_millis(1)).await;
                            }
                        }).await??;
                    }
                    survival.refresh_inventory_move(&mut parent).await?;
                    Ok(())
                } else if creative {
                    client.creative().move_inventory(&mut parent).await
                } else {
                    survival.move_inventory(&mut parent).await
                };
                serde_json::json!({"record": parent, "error": result.err().map(|e| e.to_string()),
                    "player": client.player_state().await?})
            }
            "screen" => serde_json::to_value(client.screen_state().await?)?,
            "close_screen" => {
                let screen = client.screen_state().await?.screen.ok_or_else(|| anyhow::anyhow!("no active screen"))?.id;
                serde_json::to_value(if client.player_state().await?.game_mode == Some(GameMode::Creative) {
                    client.creative().close_container(screen).await?
                } else { survival.close_container(screen).await? })?
            }
            "crafting_input_swap" => {
                use voxrig::client::inventory::InventorySource;
                let source = if request["container"] == true {
                    InventorySource::Container { screen: client.screen_state().await?.screen.ok_or_else(|| anyhow::anyhow!("no active screen"))?.id }
                } else { InventorySource::Player };
                let slot = request["slot"].as_u64().unwrap().try_into()?;
                let hotbar = request["hotbar"].as_u64().unwrap().try_into()?;
                serde_json::to_value(if client.player_state().await?.game_mode == Some(GameMode::Creative) {
                    client.creative().swap_crafting_input_hotbar(source, slot, hotbar).await?
                } else { survival.swap_crafting_input_hotbar(source, slot, hotbar).await? })?
            }
            "inventory_swap_record" => serde_json::to_value(survival.inventory_swap_record().await?)?,
            "inventory_transfer" => {
                use voxrig::client::inventory::InventorySource;
                let source = if request["container"] == true {
                    InventorySource::Container { screen: client.screen_state().await?.screen.ok_or_else(|| anyhow::anyhow!("no active screen"))?.id }
                } else { InventorySource::Player };
                let slot = request["slot"].as_u64().unwrap().try_into()?;
                serde_json::to_value(if client.player_state().await?.game_mode == Some(GameMode::Creative) {
                    client.creative().transfer_inventory(source, slot).await?
                } else { survival.transfer_inventory(source, slot).await? })?
            }
            "inventory_transfer_record" => serde_json::to_value(survival.inventory_transfer_record().await?)?,
            "inventory_record" => serde_json::to_value(survival.inventory_click_record().await?)?,
            "inventory_click" => {
                use voxrig::client::inventory::{
                    InventoryClickButton as Button, InventoryClickStage, InventorySource,
                };
                let slot = request["slot"].as_u64().unwrap().try_into()?;
                let button = if request["button"] == "right" {
                    Button::Right
                } else {
                    Button::Left
                };
                let source = if request["container"] == true {
                    InventorySource::Container { screen: client.screen_state().await?.screen.ok_or_else(|| anyhow::anyhow!("no active screen"))?.id }
                } else { InventorySource::Player };
                let mode = client.player_state().await?.game_mode;
                let result = if mode == Some(GameMode::Creative) {
                    client
                        .creative()
                        .click_inventory(source, slot, button)
                        .await
                } else {
                    survival
                        .click_inventory(source, slot, button)
                        .await
                };
                if request["expect_rejected"] == true {
                    let error =
                        result.expect_err("invalid equipment fixture unexpectedly admitted");
                    serde_json::json!({"rejected":true, "kind":format!("{:?}",error.kind()),
                        "message":error.to_string()})
                } else {
                    let pending = result?;
                    if request["submit_only"] == true {
                        serde_json::to_value(pending)?
                    } else {
                        let complete = tokio::time::timeout(Duration::from_secs(5), async {
                            loop {
                                let current = survival.inventory_click_record().await?.unwrap();
                                anyhow::ensure!(
                                    current.id == pending.id,
                                    "another click replaced original attempt"
                                );
                                match current.stage {
                                    InventoryClickStage::ObservedClicked => {
                                        break Ok::<_, anyhow::Error>(current);
                                    }
                                    InventoryClickStage::RequiresInspection => {
                                        anyhow::bail!("click requires inspection: {current:?}")
                                    }
                                    InventoryClickStage::Pending => {}
                                    _ => anyhow::bail!("unreviewed click stage"),
                                }
                                tokio::time::sleep(Duration::from_millis(10)).await;
                            }
                        })
                        .await??;
                        serde_json::to_value(complete)?
                    }
                }
            }
            "vehicle" => serde_json::to_value(client.vehicle_state().await?)?,
            "events" => serde_json::to_value(
                client
                    .events_after(request["cursor"].as_u64().unwrap_or(0))
                    .await?,
            )?,
            "player_context" => serde_json::to_value(client.player_context().await?)?,
            "chunk_context" => {
                let position = serde_json::from_value(request["position"].clone())?;
                let context = client.chunk_context(position).await?;
                if request["save"] == true {
                    saved_chunk = context.clone();
                }
                let samples = context.as_ref().and_then(|context| context.heightmaps.as_ref()).map(|maps| {
                    maps.value.iter().map(|map| {
                        let values = (0..16).flat_map(|z| (0..16).map(move |x| map.first_available_y(x,z))).collect::<Vec<_>>();
                        serde_json::json!({"kind":map.kind,"first_available_y":map.first_available_y(1,1),"values":values})
                    }).collect::<Vec<_>>()
                });
                serde_json::json!({"context":context,"heightmap_samples":samples})
            }
            "saved_chunk_context" => serde_json::to_value(&saved_chunk)?,
            "chunk_context_disconnect" => {
                client.disconnect().await?;
                tokio::time::sleep(Duration::from_millis(150)).await;
                println!(
                    "{}",
                    serde_json::json!({"context":client.chunk_context(serde_json::from_value(request["position"].clone())?).await?,"saved":saved_chunk})
                );
                std::io::stdout().flush()?;
                return Ok(());
            }
            "vehicle_record" => serde_json::to_value(client.vehicle_control_record().await?)?,
            "entities" => serde_json::to_value(client.entities().await?)?,
            "vehicle_disconnect" => {
                client.disconnect().await?;
                println!(
                    "{}",
                    serde_json::json!({"record":client.vehicle_control_record().await?})
                );
                std::io::stdout().flush()?;
                return Ok(());
            }
            "capture" => serde_json::to_value(
                client
                    .capture(Region {
                        min: [0, 64, 0],
                        max: [0, 66, 0],
                    })
                    .await?,
            )?,
            "look" => {
                survival
                    .look(serde_json::from_value(request["rotation"].clone())?)
                    .await?;
                serde_json::to_value(client.player_state().await?)?
            }
            "revoke" => {
                let revoked = client.revoke_connection();
                let rejected = client.player_state().await.is_err();
                println!(
                    "{}",
                    serde_json::json!({"revoked":revoked,"player_rejected":rejected})
                );
                std::io::stdout().flush()?;
                return Ok(());
            }
            "revoke_control" => {
                let revoked = client.revoke_connection();
                let rejected = survival
                    .request_ground_jump(request["session_id"].as_u64().unwrap())
                    .await
                    .is_err();
                let record = tokio::time::timeout(Duration::from_secs(5), async {
                    loop {
                        let record = survival.control_record().await?.unwrap();
                        if matches!(
                            record.status,
                            voxrig::client::control::ControlStatus::Stopped { .. }
                        ) {
                            break Ok::<_, anyhow::Error>(record);
                        }
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                })
                .await??;
                println!(
                    "{}",
                    serde_json::json!({"revoked": revoked, "jump_rejected": rejected, "record": record})
                );
                std::io::stdout().flush()?;
                return Ok(());
            }
            "mode_start" | "flight_start" | "glide_start" | "flight_enable" | "mode_keys" | "mode_jump"
            | "mode_stop" => {
                let mode: GameMode = serde_json::from_value(request["mode"].clone())?;
                let handle = client.player_control(mode);
                let result: voxrig::Result<serde_json::Value> = match request["command"]
                    .as_str()
                    .unwrap()
                {
                    "flight_start" => handle
                        .start_flight_control(serde_json::from_value(request["controls"].clone())?)
                        .await
                        .map(|r| serde_json::json!({"record":r})),
                    "flight_enable" => client
                        .creative()
                        .set_flying(request["flying"].as_bool().unwrap())
                        .await
                        .map(|r| serde_json::json!({"record":r})),
                    "glide_start" => handle.start_glide_control(serde_json::from_value(request["controls"].clone())?).await.map(|r| serde_json::json!({"record":r})),
                    "mode_start" => handle
                        .start_control()
                        .await
                        .map(|r| serde_json::json!({"record":r})),
                    "mode_keys" => handle
                        .set_controls_for(
                            request["session_id"].as_u64().unwrap(),
                            serde_json::from_value(request["controls"].clone())?,
                        )
                        .await
                        .map(|r| serde_json::json!({"record":r})),
                    "mode_jump" => handle
                        .request_ground_jump(request["session_id"].as_u64().unwrap())
                        .await
                        .map(|r| serde_json::json!({"request":r})),
                    _ => handle
                        .stop_control_for(request["session_id"].as_u64().unwrap())
                        .await
                        .map(|r| serde_json::json!({"record":r})),
                };
                match result {
                    Ok(mut value) => {
                        value["admitted"] = serde_json::json!(true);
                        value
                    }
                    Err(error) => {
                        serde_json::json!({"admitted":false,"kind":format!("{:?}",error.kind()),"error":error.to_string(),"record":handle.control_record().await?})
                    }
                }
            }
            "start" => serde_json::to_value(survival.start_control().await?)?,
            "jump" => {
                match survival
                    .request_ground_jump(request["session_id"].as_u64().unwrap())
                    .await
                {
                    Ok(record) => serde_json::json!({"request": record}),
                    Err(error) => serde_json::json!({"error": error.to_string()}),
                }
            }
            "restart" => {
                let stopped = survival.stop_control().await?;
                let started = survival.start_control().await?;
                let first = tokio::time::timeout(Duration::from_secs(5), async {
                    loop {
                        let record = survival.control_record().await?.unwrap();
                        if record.dispatched_ticks > 0 {
                            break Ok::<_, anyhow::Error>(record);
                        }
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                })
                .await??;
                serde_json::json!({"stopped":stopped,"started":started,"first":first})
            }
            "keys" => {
                let controls: Controls = serde_json::from_value(request["controls"].clone())?;
                serde_json::to_value(survival.set_controls(controls).await?)?
            }
            "keys_for" => {
                let controls = serde_json::from_value(request["controls"].clone())?;
                let result = survival
                    .set_controls_for(request["session_id"].as_u64().unwrap(), controls)
                    .await;
                match result {
                    Ok(record) => serde_json::json!({"admitted":true, "record":record}),
                    Err(error) => serde_json::json!({"admitted":false, "error":error.to_string(),
                        "record":survival.control_record().await?}),
                }
            }
            "ticks" => {
                let initial = survival.control_record().await?.unwrap().dispatched_ticks;
                let count = request["count"].as_u64().unwrap();
                let record = tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        let record = survival.control_record().await?.unwrap();
                        if record.dispatched_ticks >= initial + count {
                            break Ok::<_, anyhow::Error>(record);
                        }
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                })
                .await;
                let record = match record {
                    Ok(record) => record?,
                    Err(error) => anyhow::bail!(
                        "control tick wait failed: {error}; retained record: {:?}",
                        survival.control_record().await?
                    ),
                };
                serde_json::to_value(record)?
            }
            "record" => serde_json::to_value(survival.control_record().await?)?,
            "mount" => {
                let entity_type = request["type"].as_str().unwrap();
                let target = tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        if let Some(entity) = client
                            .entity_spawns()
                            .await?
                            .entities
                            .into_iter()
                            .find(|e| e.type_name.as_deref() == Some(entity_type))
                        {
                            break Ok::<_, anyhow::Error>(entity.id);
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                })
                .await??;
                if creative_vehicle {
                    client
                        .creative()
                        .interact_entity(target, Hand::Main, false)
                        .await?;
                } else {
                    survival.interact_entity(target, Hand::Main, false).await?;
                }
                let vehicle = tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        let vehicle = client.vehicle_state().await?;
                        if let Some(VehicleRelation::Mounted { mount }) =
                            vehicle.relation.as_ref().map(|r| &r.value)
                        {
                            if mount.vehicle() == Some(target) {
                                mounted = Some(*mount);
                                break Ok::<_, anyhow::Error>(vehicle);
                            }
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                })
                .await??;
                serde_json::to_value(vehicle)?
            }
            "drive_cancelled_waiter" => {
                let inputs: Vec<VehicleInput> = serde_json::from_value(request["inputs"].clone())?;
                let timed = tokio::time::timeout(Duration::from_millis(80), async {
                    if creative_vehicle {
                        client
                            .creative()
                            .start_vehicle_control(mounted.unwrap(), &inputs)
                            .await
                    } else {
                        survival
                            .start_vehicle_control(mounted.unwrap(), &inputs)
                            .await
                    }
                })
                .await;
                anyhow::ensure!(
                    timed.is_err(),
                    "finite waiter did not remain pending for cancellation"
                );
                let pending = client.vehicle_control_record().await?.unwrap();
                anyhow::ensure!(
                    pending.stage == VehicleControlStage::Running,
                    "cancelled waiter lost running owner"
                );
                let complete = tokio::time::timeout(Duration::from_secs(12), async {
                    loop {
                        let record = client.vehicle_control_record().await?.unwrap();
                        anyhow::ensure!(
                            record.id == pending.id,
                            "cancelled finite owner was replaced"
                        );
                        if record.stage != VehicleControlStage::Running {
                            break Ok::<_, anyhow::Error>(record);
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                })
                .await??;
                serde_json::json!({"cancelled_waiter": true, "pending": pending,
                    "error": complete.requires_inspection, "record": complete})
            }
            "drive" | "drive_until_interrupted" => {
                let inputs: Vec<VehicleInput> = serde_json::from_value(request["inputs"].clone())?;
                let result = if creative_vehicle {
                    client
                        .creative()
                        .start_vehicle_control(mounted.unwrap(), &inputs)
                        .await
                } else {
                    survival
                        .start_vehicle_control(mounted.unwrap(), &inputs)
                        .await
                };
                if request["command"] == "drive_until_interrupted" {
                    serde_json::json!({
                        "error": result.err().map(|e| e.to_string()),
                        "record": client.vehicle_control_record().await?,
                    })
                } else {
                    serde_json::to_value(result?)?
                }
            }
            "dismount" => {
                let record = if creative_vehicle {
                    client.creative().dismount(mounted.unwrap()).await?
                } else {
                    survival.dismount(mounted.unwrap()).await?
                };
                tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        let current = client.dismount_record().await?.unwrap();
                        if current.stage == DismountStage::ObservedUnmounted {
                            break Ok::<_, anyhow::Error>(());
                        }
                        if current.requires_inspection.is_some() {
                            anyhow::bail!("dismount interrupted");
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                })
                .await??;
                let complete = if creative_vehicle {
                    client.creative().complete_dismount(record.id).await?
                } else {
                    survival.complete_dismount(record.id).await?
                };
                let stale_result = if creative_vehicle {
                    client
                        .creative()
                        .start_vehicle_control(mounted.unwrap(), &[VehicleInput::default()])
                        .await
                } else {
                    survival
                        .start_vehicle_control(mounted.unwrap(), &[VehicleInput::default()])
                        .await
                };
                anyhow::ensure!(stale_result.is_err());
                serde_json::to_value(complete)?
            }
            "stop" => serde_json::to_value(survival.stop_control().await?)?,
            "stop_for" => {
                let result = survival
                    .stop_control_for(request["session_id"].as_u64().unwrap())
                    .await;
                match result {
                    Ok(record) => serde_json::json!({"admitted":true, "record":record}),
                    Err(error) => serde_json::json!({"admitted":false, "error":error.to_string(),
                        "record":survival.control_record().await?}),
                }
            }
            "wait" => {
                tokio::time::sleep(Duration::from_millis(request["ms"].as_u64().unwrap())).await;
                serde_json::to_value(survival.control_record().await?)?
            }
            "disconnect" => {
                client.disconnect().await?;
                tokio::time::sleep(Duration::from_millis(150)).await;
                println!(
                    "{}",
                    serde_json::to_value(survival.control_record().await?)?
                );
                std::io::stdout().flush()?;
                return Ok(());
            }
            "context_disconnect" => {
                client.disconnect().await?;
                tokio::time::sleep(Duration::from_millis(150)).await;
                println!("{}", serde_json::to_value(client.player_context().await?)?);
                std::io::stdout().flush()?;
                return Ok(());
            }
            other => anyhow::bail!("unknown command: {other}"),
        };
        println!("{value}");
        std::io::stdout().flush()?;
    }
    client.disconnect().await?;
    Ok(())
}
