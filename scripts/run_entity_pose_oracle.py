#!/usr/bin/env python3
"""Audit original pose IDs/serializers; publish interpreted results and hashes."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
from movement_oracle.run import classpath, VERSIONS

REPO=Path(__file__).resolve().parents[1]
SOURCE=REPO/'scripts/EntityPoseOracle.java'


def mappings(path):
    result={};current=None
    for line in path.read_text().splitlines():
        if line.startswith('#'):continue
        if not line.startswith(' '):
            current,obf=line.rstrip(':').split(' -> ');result[current]={'class':obf,'members':[]}
        else:result[current]['members'].append(line.strip())
    return result


def member(table,owner,text):
    matches=[m.split(' -> ')[1] for m in table[owner]['members'] if text in m]
    if len(matches)!=1:raise ValueError((owner,text,matches))
    return matches[0]


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--downloads',type=Path,required=True)
    p.add_argument('--work',type=Path,required=True)
    p.add_argument('--output',type=Path,default=REPO/'data/client_api/entity_pose_oracle.json')
    args=p.parse_args();args.work.mkdir(parents=True,exist_ok=True)
    classes=args.work/'classes';classes.mkdir(exist_ok=True)
    subprocess.run(['javac','-d',str(classes),str(SOURCE)],check=True)
    result={'schema':1,'authority':'Original pose enum, Entity.DATA_POSE accessor and original EntityDataSerializers.POSE round trips; interpreted results and hashes only. No synthesized server-current pose or geometry.','harness_sha256':hashlib.sha256(SOURCE.read_bytes()).hexdigest(),'runner_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),'versions':{}}
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
        specs=[c('world.entity.Pose'),c('network.syncher.EntityDataSerializers'),field('network.syncher.EntityDataSerializers','POSE'),c('network.syncher.EntityDataSerializer'),meth('network.syncher.EntityDataSerializers','getSerializedId'),c('network.FriendlyByteBuf') if legacy else c('network.RegistryFriendlyByteBuf'),'legacy' if legacy else 'modern','-' if legacy else c('core.RegistryAccess'),'-' if legacy else field('core.RegistryAccess','EMPTY'),meth('network.syncher.EntityDataSerializer','write') if legacy else meth('network.syncher.EntityDataSerializer','codec'),'-' if legacy else c('network.codec.StreamCodec'),'-' if legacy else meth('network.codec.StreamEncoder','encode'),meth('network.syncher.EntityDataSerializer','read') if legacy else meth('network.codec.StreamDecoder','decode'),'-' if legacy else c('SharedConstants')+'.'+meth('SharedConstants','tryDetectVersion'),c('server.Bootstrap')+'.'+meth('server.Bootstrap','bootStrap'),c('world.entity.Entity'),field('world.entity.Entity','DATA_POSE'),field('network.syncher.EntityDataAccessor','id')]
        import os
        cp=os.pathsep.join(str(v.resolve()) for v in [classes,*libs])
        output=folder/'output.txt';error=folder/'stderr.log'
        with output.open('w') as out,error.open('w') as err:
            subprocess.run(['java','-cp',cp,'voxrig.oracle.EntityPoseOracle',*specs],stdout=out,stderr=err,check=True,timeout=60,cwd=folder.resolve())
        rows=[];kind=None;key=None
        for line in output.read_text().splitlines():
            columns=line.split('\t')
            if columns[0]=='SERIALIZER':kind=int(columns[1])
            elif columns[0]=='KEY':key=int(columns[1])
            elif columns[0]=='POSE':
                ordinal,native_id,name,size,sha=columns[1:]
                rows.append(dict(ordinal=int(ordinal),native_id=int(native_id),name=name,original_roundtrip=True,encoded_size=int(size),original_codec_sha256=sha))
        assert key==6 and kind==(18 if legacy else 20)
        assert len(rows)==(7 if legacy else 18)
        result['versions'][version]=dict(original_server_jar_sha1=VERSIONS[version][0],mappings_sha1=VERSIONS[version][1],original_output_sha256=hashlib.sha256(output.read_bytes()).hexdigest(),metadata_index=key,serializer_id=kind,poses=rows)
    args.output.write_text(json.dumps(result,indent=2)+'\n')
    print('original pose round trips:',sum(len(v['poses']) for v in result['versions'].values()))
if __name__=='__main__':main()
