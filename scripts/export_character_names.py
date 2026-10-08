#!/usr/bin/env python3
"""Capture original JDK name grammar used by modern SNBT escapes."""
import argparse
import gzip
import json
import subprocess
import shutil
from pathlib import Path
from export_regular_clicks import ROOT, digest, encoded


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--runtime-output',type=Path,required=True);p.add_argument('--normalize-only',action='store_true');p.add_argument('--check',action='store_true');a=p.parse_args();b=a.runtime_output.resolve();b.mkdir(parents=True,exist_ok=True);raw=b/'raw.json'
    if not a.normalize_only:
        with (b/'run.log').open('w') as log:subprocess.run(['java','-Xmx512M','-XX:ActiveProcessorCount=1',str(ROOT/'scripts/ExportCharacterNames.java'),str(raw)],cwd=b,stdout=log,stderr=subprocess.STDOUT,check=True)
    java=Path(shutil.which('java')).resolve();modules=java.parent.parent/'lib/modules';runtime_jdk={'java_executable_sha256':digest(java.read_bytes()),'jdk_modules_sha256':digest(modules.read_bytes())};data=json.loads(raw.read_text());assert data['java_version']=='21.0.12.1';file='data/client_api/character_names-21.0.12.1.json.gz';content=gzip.compress(encoded(data,compact=True),mtime=0)
    source=encoded({'authority':'Running unchanged JDK Character.getName/codePointOf and Locale.ROOT ASCII-result case folds; factual grammar for named SNBT escapes, not Minecraft entity resolution.','java_version':data['java_version'],'runtime_jdk_sha256':runtime_jdk,'generators_sha256':{s:digest((ROOT/s).read_bytes()) for s in ['scripts/ExportCharacterNames.java','scripts/export_character_names.py','scripts/export_regular_clicks.py']},'raw_output_sha256':digest(raw.read_bytes()),'files_sha256':{file:digest(content)},'names':len(data['names']),'ascii_upper_folds':len(data['ascii_upper'])})
    for name,value in [(file,content),('data/client_api/character_names_source.json',source)]:
        path=ROOT/name
        if a.check:
            if path.read_bytes()!=value:raise SystemExit('native name facts differ: '+name)
        else:path.write_bytes(value)
    print('Original character names verified' if a.check else 'Original character names generated',len(data['names']),len(data['ascii_upper']))


if __name__=='__main__':main()
