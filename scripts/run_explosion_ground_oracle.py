#!/usr/bin/env python3
"""Run ordered explosion/velocity inputs through unchanged official Player travel.

Only our harness is extended: dimension origin, initial velocity, box observations,
and explicit ordered motion inputs before travel. Original game methods are never replaced.
"""
import argparse
import gzip
import hashlib
import json
import sys
import re
import subprocess
from pathlib import Path
from movement_oracle import run as original
from run_player_scale_oracle import replace_once

REPO = Path(__file__).resolve().parents[1]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--downloads', type=Path, required=True)
    parser.add_argument('--work', type=Path, required=True)
    parser.add_argument('--scenarios', type=Path, required=True)
    parser.add_argument('--output', type=Path, default=REPO/'data/client_api/explosion_ground_oracle.json.gz')
    args = parser.parse_args()
    client = args.downloads/'1.21.11-client.jar'
    mappings = args.downloads/'1.21.11-client-mappings.txt'
    assert hashlib.sha1(client.read_bytes()).hexdigest() == 'ba2df812c2d12e0219c489c4cd9a5e1f0760f5bd'
    assert hashlib.sha1(mappings.read_bytes()).hexdigest() == '031a68bebf55d824f66d6573d8c752f0e1bf232a'
    named_client = args.work.resolve()/'client-named.jar'
    named_client.parent.mkdir(parents=True, exist_ok=True)
    tool = args.downloads/'SpecialSource-1.11.4-shaded.jar'
    assert sha(tool) == original.SPECIAL_SOURCE_SHA256
    subprocess.run(['java', '-Xmx512M', '-XX:ActiveProcessorCount=1', '-jar', str(tool),
                    '--in-jar', str(client), '--out-jar', str(named_client),
                    '--srg-in', str(mappings)], check=True, stdout=subprocess.DEVNULL)
    handler = subprocess.check_output(['javap', '-v', '-p', '-classpath', str(named_client),
                                      'net.minecraft.client.multiplayer.ClientPacketListener'], text=True)
    assert 'REF_invokeVirtual net/minecraft/world/entity/Entity.addDeltaMovement:' in handler
    fingerprints = {}
    for owner, names in [('net.minecraft.client.multiplayer.ClientPacketListener', ['handleExplosion', 'handleSetEntityMotion']),
                         ('net.minecraft.world.entity.Entity', ['addDeltaMovement', 'lerpMotion'])]:
        raw = subprocess.check_output(['javap', '-c', '-p', '-classpath', str(named_client), owner], text=True)
        for name in names:
            blocks = [b for b in re.split(r'\n(?=  [^ ])', raw) if re.search(r' '+name+r'\(', b.split('\n')[0])]
            assert len(blocks) == 1, name
            fingerprints[owner+'.'+name] = hashlib.sha256(blocks[0].encode()).hexdigest()
    generated = args.work.resolve()/'generated/java_1_21_11'
    generated.mkdir(parents=True, exist_ok=True)
    base = REPO/'scripts/movement_oracle/java_1_21_11/MovementOracle.java'
    source = base.read_text()
    needle = '        JsonObject initial = new JsonObject();'
    source = replace_once(source, needle, '''        player.refreshDimensions();
        if (scenario.has("velocity")) {
            JsonArray v = scenario.getAsJsonArray("velocity");
            player.setDeltaMovement(v.get(0).getAsDouble(), v.get(1).getAsDouble(), v.get(2).getAsDouble());
        }
        player.fallDistance = 0.;
''' + needle + '''
        initial.addProperty("scale_attribute", Double.toString(player.getAttributeValue(net.minecraft.world.entity.ai.attributes.Attributes.SCALE)));''')
    needle = '            f.addProperty("speed", Float.toString(player.getSpeed()));'
    source = replace_once(source, needle, needle + '''
            f.addProperty("eye_height", Float.toString(player.getEyeHeight()));
            AABB box = player.getBoundingBox();
            f.add("bounds", vec(box.minX-origin.getX(), box.minY-origin.getY(), box.minZ-origin.getZ()));
            f.add("bounds_max", vec(box.maxX-origin.getX(), box.maxY-origin.getY(), box.maxZ-origin.getZ()));''')
    needle = '            player.tick();'
    source = replace_once(source, needle, '''            if (t.has("motion_inputs")) {
                for (var input : t.getAsJsonArray("motion_inputs")) {
                    var motion = input.getAsJsonObject();
                    if (motion.get("value").isJsonNull()) continue;
                    var v = motion.getAsJsonArray("value");
                    var value = new Vec3(v.get(0).getAsDouble(), v.get(1).getAsDouble(), v.get(2).getAsDouble());
                    // Original ClientPacketListener.handleExplosion binds Entity.addDeltaMovement;
                    // handleSetEntityMotion calls Entity.lerpMotion. Neither body is replaced.
                    if (motion.get("kind").getAsString().equals("explosion")) player.addDeltaMovement(value);
                    else player.lerpMotion(value);
                }
            }
''' + needle)
    target = generated/'MovementOracle.java'

    target.write_text(source)
    original.HERE = generated.parent
    sys.argv = [str(original.__file__), '--downloads', str(args.downloads.resolve()),
                '--work', str(args.work.resolve()/'native'), '--version', '1.21.11',
                '--scenarios', str(args.scenarios.resolve()), '--output', str(args.output.resolve()),
                '--blocks-output', str(args.work.resolve()/'blocks')]
    original.main()
    fixture = json.loads(gzip.decompress(args.output.read_bytes()))
    fixture['handler_audit'] = dict(original_client_jar_sha1=hashlib.sha1(client.read_bytes()).hexdigest(), client_mappings_sha1=hashlib.sha1(mappings.read_bytes()).hexdigest(), method_sha256=fingerprints, rule='handleExplosion Optional.ifPresent binds original Entity.addDeltaMovement; handleSetEntityMotion invokes original Entity.lerpMotion. These exact methods supply numerical inputs to unchanged official travel.')
    fixture['owned_sources'] = dict(base_harness_sha256=sha(base), generated_harness_sha256=sha(target), wrapper_sha256=sha(Path(__file__)))
    args.output.write_bytes(gzip.compress((json.dumps(fixture, separators=(',', ':'))+'\n').encode(), mtime=0))


if __name__ == '__main__':
    main()
