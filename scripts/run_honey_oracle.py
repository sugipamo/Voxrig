#!/usr/bin/env python3
"""Run modern honey scenarios through unchanged official Player travel/hooks.

Only our harness is extended: dimension origin, initial velocity/fall distance,
scale and box observations. Original game methods are never replaced.
"""
import argparse
import gzip
import hashlib
import json
import sys
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
    parser.add_argument('--output', type=Path, default=REPO/'data/client_api/honey_oracle.json.gz')
    args = parser.parse_args()
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
        player.fallDistance = scenario.get("fall_distance").getAsDouble();
''' + needle + '''
        initial.addProperty("scale_attribute", Double.toString(player.getAttributeValue(net.minecraft.world.entity.ai.attributes.Attributes.SCALE)));''')
    needle = '            f.addProperty("speed", Float.toString(player.getSpeed()));'
    source = replace_once(source, needle, needle + '''
            f.addProperty("eye_height", Float.toString(player.getEyeHeight()));
            AABB box = player.getBoundingBox();
            f.add("bounds", vec(box.minX-origin.getX(), box.minY-origin.getY(), box.minZ-origin.getZ()));
            f.add("bounds_max", vec(box.maxX-origin.getX(), box.maxY-origin.getY(), box.maxZ-origin.getZ()));''')
    target = generated/'MovementOracle.java'
    target.write_text(source)
    original.HERE = generated.parent
    sys.argv = [str(original.__file__), '--downloads', str(args.downloads.resolve()),
                '--work', str(args.work.resolve()/'native'), '--version', '1.21.11',
                '--scenarios', str(args.scenarios.resolve()), '--output', str(args.output.resolve()),
                '--blocks-output', str(args.work.resolve()/'blocks')]
    original.main()
    fixture = json.loads(gzip.decompress(args.output.read_bytes()))
    fixture['owned_sources'] = dict(base_harness_sha256=sha(base), generated_harness_sha256=sha(target), wrapper_sha256=sha(Path(__file__)))
    args.output.write_bytes(gzip.compress((json.dumps(fixture, separators=(',', ':'))+'\n').encode(), mtime=0))


if __name__ == '__main__':
    main()
