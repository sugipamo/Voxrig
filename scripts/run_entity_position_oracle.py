#!/usr/bin/env python3
"""Audit original position serializers; publish interpreted results and hashes."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
from movement_oracle.run import classpath, VERSIONS

REPO=Path(__file__).resolve().parents[1]
SOURCE=REPO/'scripts/EntityPositionOracle.java'


from run_entity_pose_oracle import mappings, member

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--downloads',type=Path,required=True)
    p.add_argument('--work',type=Path,required=True)
    p.add_argument('--output',type=Path,default=REPO/'data/client_api/entity_position_oracle.json')
    args=p.parse_args();args.work.mkdir(parents=True,exist_ok=True)
    classes=args.work/'classes';classes.mkdir(exist_ok=True)
    subprocess.run(['javac','-d',str(classes),str(SOURCE)],check=True)
    result={'schema':1,'authority':'Original EntityDataSerializers block/optional-block-position round trips; decoded signed coordinates and hashes only. No server-current state or tick authority.','harness_sha256':hashlib.sha256(SOURCE.read_bytes()).hexdigest(),'runner_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),'versions':{}}
    for version in ('1.16.1','1.21.11'):
        folder=args.work/version;folder.mkdir(exist_ok=True)
        jar=args.downloads/f'{version}-server.jar';mapping=args.downloads/f'{version}-server-mappings.txt'
        assert hashlib.sha1(jar.read_bytes()).hexdigest()==VERSIONS[version][0]
        assert hashlib.sha1(mapping.read_bytes()).hexdigest()==VERSIONS[version][1]
        table=mappings(mapping);legacy=version=='1.16.1'
        game,libs=classpath(version,jar,folder)
        c=lambda suffix:table['net.minecraft.'+suffix]['class']
        field=lambda owner,name:member(table,'net.minecraft.'+owner,' '+name+' ->')
        meth=lambda owner,name:member(table,'net.minecraft.'+owner,name+'(')
        specs=[c('core.BlockPos'),c('network.syncher.EntityDataSerializers'),field('network.syncher.EntityDataSerializers','BLOCK_POS'),c('network.syncher.EntityDataSerializer'),meth('network.syncher.EntityDataSerializers','getSerializedId'),c('network.FriendlyByteBuf') if legacy else c('network.RegistryFriendlyByteBuf'),'legacy' if legacy else 'modern','-' if legacy else c('core.RegistryAccess'),'-' if legacy else field('core.RegistryAccess','EMPTY'),meth('network.syncher.EntityDataSerializer','write') if legacy else meth('network.syncher.EntityDataSerializer','codec'),'-' if legacy else c('network.codec.StreamCodec'),'-' if legacy else meth('network.codec.StreamEncoder','encode'),meth('network.syncher.EntityDataSerializer','read') if legacy else meth('network.codec.StreamDecoder','decode'),'-' if legacy else c('SharedConstants')+'.'+meth('SharedConstants','tryDetectVersion'),c('server.Bootstrap')+'.'+meth('server.Bootstrap','bootStrap'),c('world.entity.LivingEntity'),field('world.entity.LivingEntity','SLEEPING_POS_ID'),field('network.syncher.EntityDataAccessor','id')]
        import os
        cp=os.pathsep.join(str(v.resolve()) for v in [classes,*libs])
        kinds=[]
        specs.extend([field('network.syncher.EntityDataSerializers','OPTIONAL_BLOCK_POS'),meth('core.Vec3i','getX'),meth('core.Vec3i','getY'),meth('core.Vec3i','getZ')])
        for optional in (False,True):
            specs[2]=field('network.syncher.EntityDataSerializers','OPTIONAL_BLOCK_POS' if optional else 'BLOCK_POS')
            label='optional' if optional else 'position'
            output=folder/(label+'-output.txt');error=folder/(label+'-stderr.log')
            with output.open('w') as out,error.open('w') as err:
                subprocess.run(['java','-cp',cp,'voxrig.oracle.EntityPositionOracle',*specs],stdout=out,stderr=err,check=True,timeout=60,cwd=folder.resolve())
            rows=[];kind=None;key=None
            for line in output.read_text().splitlines():
                columns=line.split('\t')
                if columns[0]=='SERIALIZER':kind=int(columns[1])
                elif columns[0]=='KEY':key=int(columns[1])
                elif columns[0]=='POSITION':
                    source,decoded,size,sha=columns[1:]
                    parse=lambda value: None if value=='null' else [int(n) for n in value.split(',')]
                    rows.append(dict(input=parse(source),decoded=parse(decoded),encoded_size=int(size),original_codec_sha256=sha))
            assert key==(13 if legacy else 14) and kind==((10 if legacy else 11) if optional else (9 if legacy else 10))
            assert len(rows)==(72 if optional else 71)
            kinds.append(dict(optional=optional,serializer_id=kind,original_output_sha256=hashlib.sha256(output.read_bytes()).hexdigest(),truncated_boundaries_rejected=sum(r["encoded_size"] for r in rows),nonzero_presence_roundtrips=71 if optional else 0,positions=rows))
        result['versions'][version]=dict(original_server_jar_sha1=VERSIONS[version][0],mappings_sha1=VERSIONS[version][1],sleeping_metadata_index=key,codecs=kinds)
    args.output.write_text(json.dumps(result,indent=2)+'\n')
    print('original position round trips:',sum(len(c['positions']) for v in result['versions'].values() for c in v['codecs']))
if __name__=='__main__':main()
