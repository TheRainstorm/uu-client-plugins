#!/usr/bin/env python3
"""Build the independently distributed extension against its pinned engine SDK."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import zipfile
import sdk_workspace
import validate

ROOT=Path(__file__).resolve().parents[1]
ENGINE=ROOT.parent


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target',default='x86_64-pc-windows-gnu')
    parser.add_argument('--validate-only',action='store_true')
    parser.add_argument('--features',nargs='*',default=[])
    parser.add_argument('--output',type=Path,default=ROOT/'artifacts')
    args=parser.parse_args()
    pinned=json.loads((ROOT/'engine-sdk.json').read_text())
    actual=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ENGINE,text=True).strip()
    if actual!=pinned['revision']:raise RuntimeError('engine checkout differs from pinned SDK')
    for repository in [ENGINE,ROOT]:
        paths=['crates','vendor','Cargo.toml','Cargo.lock','.cargo','tools/release/plugin_sdk.py','tools/native/plugin_probe.rs'] if repository==ENGINE else ['.']
        dirty=subprocess.check_output(['git','status','--porcelain','--untracked-files=all','--',*paths],cwd=repository,text=True)
        if dirty.strip():raise RuntimeError('commit all build inputs before packaging')
    spec=importlib.util.spec_from_file_location('extension_sdk',ENGINE/'tools/release/plugin_sdk.py')
    sdk=importlib.util.module_from_spec(spec);spec.loader.exec_module(sdk)
    main_sdk,main_standard,main_metadata=sdk.build(ENGINE,args.target,['uu-gui','uu-cli'],args.features)
    workspace,environment=sdk_workspace.prepare(ENGINE,ROOT,args.target)
    sdk.verify_consumer(ENGINE,workspace,args.target,args.features)
    library,standard,metadata=sdk.build(workspace,args.target,['uu-gui','uu-cli','uu-compat'],args.features,environment)
    target=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1','--locked'],cwd=workspace,env=environment,text=True))['target_directory']
    extension=Path(target)/args.target/'release'/('uu_compat.dll' if 'windows' in args.target else 'libuu_compat.so')
    if main_metadata['sdk_id']!=metadata['sdk_id']:raise RuntimeError('plugin SDK identity differs from the main application build')
    validate.run(ENGINE,extension,main_sdk,main_standard,main_metadata,args.target,sdk)
    if args.validate_only:return
    revision=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip()
    manifest={'api_version':metadata['api_version'],'sdk_id':metadata['sdk_id'],'id':'uu','name':'UU compatibility',
              'version':pinned['version'],'library':extension.name,'library_sha256':hashlib.file_digest(extension.open('rb'),'sha256').hexdigest(),
              'data_files':['state.json','signing-key','host.json','client-settings.json']}
    args.output.mkdir(parents=True,exist_ok=True)
    archive=args.output/f'eclipse-plugin-uu-{args.target}-{revision[:12]}.zip'
    # Extract this archive at the application root. It includes no application,
    # SDK, account files or compiler runtime; those belong to the main package.
    with zipfile.ZipFile(archive,'w',compression=zipfile.ZIP_DEFLATED) as bundle:
        bundle.writestr('plugins/uu/plugin.json',json.dumps(manifest,ensure_ascii=False,indent=2)+'\n')
        bundle.write(extension,'plugins/uu/'+extension.name)
        for name in ['README.md','LICENSE','LICENSE-MIT','LICENSE-APACHE','THIRD-PARTY-NOTICES.txt']:
            bundle.write(ROOT/name,'plugins/uu/'+name)
        bundle.write(ENGINE/'THIRD-PARTY-NOTICES.txt','plugins/uu/ENGINE-THIRD-PARTY-NOTICES.txt')
    print(json.dumps({'archive':str(archive),'plugin_revision':revision,'engine_revision':actual,'sdk_id':metadata['sdk_id'],'sha256':hashlib.file_digest(archive.open('rb'),'sha256').hexdigest()},indent=2))


if __name__=='__main__':main()
