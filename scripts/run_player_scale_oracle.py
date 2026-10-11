#!/usr/bin/env python3
"""Qualify admitted player scale with unchanged official movement methods.

The original owned movement harness remains unchanged for historical fixtures.
This wrapper adds owned observations and an explicit dimension origin to a
private generated copy, recording every source and compiled harness hash.
"""
import argparse
import gzip
import hashlib
import json
import sys
from pathlib import Path
from movement_oracle import run as original

REPO=Path(__file__).resolve().parents[1]
def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def replace_once(source,needle,addition):
    assert source.count(needle)==1,needle
    return source.replace(needle,addition)

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--downloads',type=Path,required=True)
    p.add_argument('--work',type=Path,required=True)
    p.add_argument('--scenarios',type=Path,default=REPO/'scripts/movement_oracle/player_scale_scenarios.json')
    p.add_argument('--output',type=Path,default=REPO/'data/client_api/player_scale_oracle.json.gz')
    a=p.parse_args();work=a.work.resolve();generated=work/'generated_owned_harness';generated.mkdir(parents=True,exist_ok=True)
    sources={}
    for version,(_,_,directory) in original.VERSIONS.items():
        base=REPO/'scripts/movement_oracle'/directory/'MovementOracle.java';text=base.read_text()
        # No native game code is substituted. These are additions to our owned
        # harness, calling the original public methods for the admitted origin.
        needle='        JsonObject initial = new JsonObject();'
        text=replace_once(text,needle,'        player.refreshDimensions(); // Explicit admitted dimension origin.\n'+needle)
        needle='        initial.addProperty("food", player.getFoodData().getFoodLevel());'
        observations='        initial.addProperty("scale", '+('Float.toString(player.getScale())' if version=='1.21.11' else '"1.0"')+');\n'
        if version=='1.21.11':observations+='        initial.addProperty("scale_attribute", Double.toString(player.getAttributeValue(net.minecraft.world.entity.ai.attributes.Attributes.SCALE)));\n'
        text=replace_once(text,needle,observations+needle)
        needle='            f.addProperty("speed", Float.toString(player.getSpeed()));'
        observations='''
            f.addProperty("eye_height", Float.toString(player.getEyeHeight()));
            AABB scaleBox=player.getBoundingBox();
            f.add("bounds", vec(scaleBox.minX-origin.getX(), scaleBox.minY-origin.getY(), scaleBox.minZ-origin.getZ()));
            f.add("bounds_max", vec(scaleBox.maxX-origin.getX(), scaleBox.maxY-origin.getY(), scaleBox.maxZ-origin.getZ()));'''
        text=replace_once(text,needle,needle+observations)
        folder=generated/directory;folder.mkdir(exist_ok=True);target=folder/'MovementOracle.java';target.write_text(text)
        sources[version]=dict(base_harness_path=str(base.relative_to(REPO)),base_harness_sha256=sha(base),generated_harness_sha256=sha(target))
    blocks=work/'original_blocks';blocks.mkdir(exist_ok=True)
    original.HERE=generated
    arguments=sys.argv
    try:
        sys.argv=[str(original.__file__),'--downloads',str(a.downloads.resolve()),'--work',str(work/'native'),'--scenarios',str(a.scenarios.resolve()),'--output',str(a.output.resolve()),'--blocks-output',str(blocks)]
        original.main()
    finally:sys.argv=arguments
    fixture=json.loads(gzip.decompress(a.output.read_bytes()))
    fixture['scale_origin']='Original refreshDimensions after supplied attributes/modifiers and before numerical ticks. Original block/pose/travel methods and the existing owned LocalPlayer input port run unchanged. This is an admitted numerical origin, not a native packet-clock or server acceptance.'
    fixture['wrapper_sha256']=sha(Path(__file__))
    fixture['owned_sources']=sources
    a.output.write_bytes(gzip.compress((json.dumps(fixture,separators=(',',':'))+'\n').encode(),mtime=0))
if __name__=='__main__':main()
