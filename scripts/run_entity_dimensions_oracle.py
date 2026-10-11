#!/usr/bin/env python3
"""Compare original dimension methods in registered official worlds; results/hashes only."""
import argparse
import json
import os
import shutil
from pathlib import Path
from movement_oracle.run import classpath, digest, run, read_mappings, VERSIONS, SPECIAL_SOURCE_SHA256, PROPERTIES

REPO=Path(__file__).resolve().parents[1]

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--downloads',type=Path,required=True)
    p.add_argument('--work',type=Path,required=True)
    p.add_argument('--output',type=Path,default=REPO/'data/client_api/entity_dimensions_oracle.json')
    args=p.parse_args();args.work.mkdir(parents=True,exist_ok=True)
    tool=(args.downloads/'SpecialSource-1.11.4-shaded.jar').resolve()
    assert digest(tool.read_bytes())==SPECIAL_SOURCE_SHA256
    result={'schema':1,'authority':'Original LivingEntity/Player.getDimensions and EntityDimensions.makeBoundingBox in original registered ServerLevel; modeled bounds are neither interpolated nor collision acknowledgements.','versions':{}}
    for port,(version,(sha,msha,_)) in enumerate(VERSIONS.items(),start=25710):
        work=(args.work/version).resolve();work.mkdir(parents=True,exist_ok=True)
        (work/'classes').mkdir(exist_ok=True)
        jar=(args.downloads/f'{version}-server.jar').resolve();mapping=(args.downloads/f'{version}-server-mappings.txt').resolve()
        assert digest(jar.read_bytes(),'sha1')==sha and digest(mapping.read_bytes(),'sha1')==msha
        game,cp=classpath(version,jar,work)
        named=work/'named.jar'
        run(['java','-jar',str(tool),'--in-jar',str(game),'--out-jar',str(named),'--srg-in',str(mapping)],work,work/'remap.log')
        source=REPO/'scripts/entity_dimensions_oracle'/version/'EntityDimensionsOracle.java'
        run(['javac','-nowarn','--release','21','-d',str(work/'classes'),'-cp',os.pathsep.join(str(p) for p in [named,*cp[1:]]),str(source)],work,work/'javac.log')
        run(['jar','cf',str(work/'harness-named.jar'),'-C',str(work/'classes'),'voxrig'],work,work/'jar.log')
        run(['java','-cp',f'{tool}:{named}','net.md_5.specialsource.SpecialSource','--live','--in-jar',str(work/'harness-named.jar'),'--out-jar',str(work/'harness.jar'),'--srg-in',str(mapping),'--reverse'],work,work/'unmap.log')
        server=work/'server';server.mkdir(exist_ok=True)
        (server/'eula.txt').write_text('eula=true\n')
        (server/'server.properties').write_text(PROPERTIES.format(port=port,level_type='flat' if version=='1.16.1' else 'minecraft\\:flat'))
        output=work/'original-results.json'
        methods=read_mappings(mapping)[1]
        extra=[methods[('net.minecraft.world.entity.Entity','getBoundingBoxForPose',1)][0]] if version=='1.16.1' else []
        run(['java','-Xmx1G','--add-opens','java.base/java.lang=ALL-UNNAMED','-cp',os.pathsep.join(str(p) for p in [*cp,work/'harness.jar']),'voxrig.oracle.EntityDimensionsOracle',str(output),*extra],server,work/'server.log')
        rows=json.loads(output.read_text());assert len(rows)==(49 if version=='1.16.1' else 882)
        result['versions'][version]=dict(original_server_jar_sha1=sha,mappings_sha1=msha,original_output_sha256=digest(output.read_bytes()),harness_sha256=digest(source.read_bytes()),results=rows)
    result['runner_sha256']=digest(Path(__file__).read_bytes())
    args.output.write_text(json.dumps(result,indent=2)+'\n')
    print('original dimension cases:',sum(len(v['results']) for v in result['versions'].values()))
if __name__=='__main__':main()
