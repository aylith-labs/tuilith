"""Build the public static library home and an exact pinned source download."""
import argparse, hashlib, json, os, pathlib, re, shutil, zipfile

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent

def build(base, output):
    if base not in ('', '/tuilith'):
        raise ValueError('Supported bases are empty (custom domain) and /tuilith (repository Pages)')
    output = output.resolve()
    if output == ROOT or ROOT in output.parents:
        raise ValueError('Output must be outside the source tree')
    manifest = json.loads((HERE / 'source-manifest.json').read_text())
    revision = manifest['commit']
    if not re.fullmatch('[a-f0-9]{40}', revision):
        raise ValueError('Expected exact source revision')
    contents = []
    for row in manifest['files']:
        path = pathlib.PurePosixPath(row['path'])
        if path.is_absolute() or '..' in path.parts or any(x in ('.git', '.env', 'node_modules', 'target') for x in path.parts):
            raise ValueError('Forbidden source archive path')
        source = ROOT / path
        data = os.readlink(source).encode() if source.is_symlink() else source.read_bytes()
        if hashlib.sha256(data).hexdigest() != row['sha256']:
            raise ValueError('Pinned source byte mismatch: ' + str(path))
        blob = hashlib.sha1(b'blob ' + str(len(data)).encode() + b'\0' + data).hexdigest()
        if blob != row['gitBlob']:
            raise ValueError('Pinned Gitblob mismatch: ' + str(path))
        contents.append((row, data))
    if len(contents) != 33 or len({r['path'] for r, _ in contents}) != 33:
        raise ValueError('Incomplete or duplicate canonical source inventory')
    if output.exists():
        raise ValueError('Refusing to replace an existing output; choose a fresh directory')
    assets = output / 'assets'; assets.mkdir(parents=True)
    downloads = output / 'downloads'; downloads.mkdir()
    archive = 'tuilith-source-' + revision[:12] + '.zip'
    with zipfile.ZipFile(downloads / archive, 'w', compression=zipfile.ZIP_DEFLATED) as zipped:
        for row, data in contents:
            info = zipfile.ZipInfo('tuilith/' + row['path'], (2026, 10, 3, 0, 0, 0))
            info.create_system = 3
            info.external_attr = int(row['mode'], 8) << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            zipped.writestr(info, data)
        info = zipfile.ZipInfo('tuilith/SOURCE.json', (2026, 10, 3, 0, 0, 0))
        info.create_system = 3
        info.external_attr = 0o100644 << 16
        info.compress_type = zipfile.ZIP_DEFLATED
        zipped.writestr(info, json.dumps(manifest, indent=2) + '\n')
    digest = hashlib.sha256((downloads / archive).read_bytes()).hexdigest()
    (downloads / (archive + '.sha256')).write_text(digest + '  ' + archive + '\n')
    html = (HERE / 'index.html').read_text().replace('@@BASE@@', base).replace('@@REV@@', revision).replace('@@ARCHIVE@@', archive)
    for asset in ['site.css', 'site.js']:
        version = hashlib.sha256((HERE / asset).read_bytes()).hexdigest()[:12]
        html = html.replace('/assets/' + asset, '/assets/' + asset + '?v=' + version)
    if '@@' in html:
        raise ValueError('Unresolved website template field')
    (output / 'index.html').write_text(html)
    (output / 'home.html').write_text(html)
    (output / 'home').mkdir(); (output / 'home/index.html').write_text(html)
    for name, target in [('site.css', 'site.css'), ('site.js', 'site.js'), ('favicon.svg', 'favicon.svg')]:
        shutil.copyfile(HERE / name, assets / target)
    (output / '.nojekyll').write_text('')
    (output / 'build.json').write_text(json.dumps({'base': base, 'source': revision, 'archive': archive, 'sha256': digest}, indent=2) + '\n')
    print(json.dumps({'output': str(output), 'base': base, 'archive': archive, 'sha256': digest}))

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--base', default='')
    parser.add_argument('--output', type=pathlib.Path, required=True)
    args = parser.parse_args()
    build(args.base, args.output)
