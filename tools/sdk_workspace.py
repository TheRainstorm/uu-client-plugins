"""Mirror the SDK workspace layout without editing the engine checkout.

Cargo includes workspace-relative source paths in native crate identities.
An extension cannot rebuild shared types from ../crates and expect to link to
types built as crates/... in the application. Match both layout and features.
"""
import os
from pathlib import Path
import shutil
import subprocess


def prepare(engine,plugin,target=None):
    workspace=plugin/'target/sdk-workspace'
    if workspace.exists():shutil.rmtree(workspace)
    workspace.mkdir(parents=True)
    for name in ['crates','vendor','assets','.cargo']:
        shutil.copytree(engine/name,workspace/name,ignore=shutil.ignore_patterns('target','__pycache__'))
    manifest=(engine/'Cargo.toml').read_text().replace('members = [','members = [\n    "plugins/uu-compat",',1).replace('exclude = ["plugins", ','exclude = [')
    (workspace/'Cargo.toml').write_text(manifest)
    shutil.copy2(plugin/'engine-build.lock',workspace/'Cargo.lock')
    shutil.copytree(plugin/'uu-compat',workspace/'plugins/uu-compat',ignore=shutil.ignore_patterns('target'))
    environment=os.environ.copy()
    environment['ECLIPSE_PLUGIN_ENGINE_SOURCE']=str(engine)
    environment['CARGO_TARGET_DIR']=str(plugin/'target/sdk-artifacts')
    if target is None:target=subprocess.check_output(['rustc','-vV'],text=True).split('host: ',1)[1].splitlines()[0]
    flags=['-C','prefer-dynamic']
    if 'linux' in target:flags+=['-C','link-arg=-Wl,-rpath,$ORIGIN']
    # Cargo concatenates arrays from ancestor .cargo directories. An explicit
    # encoding keeps nested build workspaces identical to the host build.
    environment['CARGO_ENCODED_RUSTFLAGS']='\x1f'.join(flags)
    environment.pop('RUSTFLAGS',None)
    return workspace,environment
