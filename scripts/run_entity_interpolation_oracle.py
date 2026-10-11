#!/usr/bin/env python3
"""Qualify explicit interpolation against unchanged official living routines."""
import argparse
import gzip
import json
import os
from pathlib import Path
from run_entity_pose_oracle import mappings, member
from movement_oracle.run import classpath, digest, run, VERSIONS, SPECIAL_SOURCE_SHA256, PROPERTIES
REPO=Path(__file__).resolve().parents[1]
CLIENT=('c9abbe8ee4fa490751ca70635340b7cf00db83ff','ddf517a4f6750f4c15189de4e03246ae1f916cf5')

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--downloads',type=Path,required=True)
    p.add_argument('--client-downloads',type=Path,required=True)
    p.add_argument('--work',type=Path,required=True)
    p.add_argument('--output',type=Path,default=REPO/'data/client_api/entity_interpolation_oracle.json.gz')
    args=p.parse_args();args.work.mkdir(parents=True,exist_ok=True)
    tool=(args.downloads/'SpecialSource-1.11.4-shaded.jar').resolve()
    assert digest(tool.read_bytes())==SPECIAL_SOURCE_SHA256
    result={'schema':1,'authority':'Unchanged original legacy client LivingEntity.lerpTo/aiStep in a headless original registered server world (AI/travel isolated), and modern original living InterpolationHandler with original Level.noCollision. Explicit numerical schedule, not server-current motion or packet dispatch.','versions':{}}
    for port,(version,(sha,msha,_)) in enumerate(VERSIONS.items(),start=25740):
        work=(args.work/version).resolve();work.mkdir(parents=True,exist_ok=True);(work/'classes').mkdir(exist_ok=True)
        jar=(args.downloads/f'{version}-server.jar').resolve();mapping=(args.downloads/f'{version}-server-mappings.txt').resolve()
        assert digest(jar.read_bytes(),'sha1')==sha and digest(mapping.read_bytes(),'sha1')==msha
        game,cp=classpath(version,jar,work)
        legacy=version=='1.16.1';extra=[]
        if legacy:
            game=(args.client_downloads/f'{version}-client.jar').resolve();mapping=(args.client_downloads/f'{version}-client-mappings.txt').resolve()
            assert digest(game.read_bytes(),'sha1')==CLIENT[0] and digest(mapping.read_bytes(),'sha1')==CLIENT[1]
            cp=[game,*cp]
        else:
            table=mappings(mapping)
            extra=[member(table,'net.minecraft.world.entity.InterpolationHandler',' interpolationData ->'),member(table,'net.minecraft.world.entity.InterpolationHandler$InterpolationData',' steps ->'),table['net.minecraft.world.entity.InterpolationHandler$InterpolationData']['class'].split('$')[-1],member(table,'net.minecraft.world.entity.InterpolationHandler',' previousTickPosition ->')]
        named=work/'named.jar'
        run(['java','-jar',str(tool),'--in-jar',str(game),'--out-jar',str(named),'--srg-in',str(mapping)],work,work/'remap.log')
        source=REPO/'scripts/entity_interpolation_oracle'/version/'EntityInterpolationOracle.java'
        run(['javac','-nowarn','--release','21','-d',str(work/'classes'),'-cp',os.pathsep.join(str(p) for p in [named,*cp[1:]]),str(source)],work,work/'javac.log')
        run(['jar','cf',str(work/'harness-named.jar'),'-C',str(work/'classes'),'voxrig'],work,work/'jar.log')
        run(['java','-cp',f'{tool}:{named}','net.md_5.specialsource.SpecialSource','--live','--in-jar',str(work/'harness-named.jar'),'--out-jar',str(work/'harness.jar'),'--srg-in',str(mapping),'--reverse'],work,work/'unmap.log')
        server=work/'server';server.mkdir(exist_ok=True);(server/'eula.txt').write_text('eula=true\n')
        (server/'server.properties').write_text(PROPERTIES.format(port=port,level_type='flat' if legacy else 'minecraft\\:flat'))
        output=work/'original-results.json'
        run(['java','-Xmx1G','--add-opens','java.base/java.lang=ALL-UNNAMED','-cp',os.pathsep.join(str(p) for p in [*cp,work/'harness.jar']),'voxrig.oracle.EntityInterpolationOracle',str(output),*extra],server,work/'server.log')
        rows=json.loads(output.read_text());assert len(rows)==210
        if not legacy:
            queries=[e['collision_free'] for r in rows for e in r['events'] if 'collision_free' in e]
            assert any(queries) and not all(queries)
        result['versions'][version]=dict(original_server_jar_sha1=sha,server_mappings_sha1=msha,original_client_jar_sha1=CLIENT[0] if legacy else None,client_mappings_sha1=CLIENT[1] if legacy else None,original_output_sha256=digest(output.read_bytes()),harness_sha256=digest(source.read_bytes()),results=rows)
    result['runner_sha256']=digest(Path(__file__).read_bytes())
    args.output.write_bytes(gzip.compress((json.dumps(result,separators=(',', ':'))+'\n').encode(),mtime=0))
    print('original interpolation schedules:',sum(len(v['results']) for v in result['versions'].values()))
if __name__=='__main__':main()
