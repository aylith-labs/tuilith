# Tuilith public homepage builder

This ships static marketing and pinned public library source, not an authenticated application or a crates.io release. Both root and `/home` render the complete shared page. `/home/` is included as well. Source archive bytes are checked against `source-manifest.json` before packaging; the original dual licenses, provenance and lockfile are retained. No toolchain or compiled dependency is shipped in this source ZIP.

Build into fresh directories outside the repository:

```sh
python3 website/build.py --base '' --output /tmp/tuilith-root
python3 website/build.py --base /tuilith --output /tmp/tuilith-prefix
```

Use `/tuilith` for the repository Pages URL. Root output is appropriate only after the repository Pages custom domain is actually `tuilith.aylith.com`. The workflow checks this binding before root selection and again before explicit deployment. An absent, mismatched or unreadable binding fails closed. Push builds prepare an artifact only; publishing requires an explicit workflow dispatch. No domain is assumed configured by this source change.

Review the output download checksum, complete root/home content, asset paths and consumer installation before deployment. Then configure the actual Pages binding, add the authorized DNS record, publish the selected matching base and verify trusted HTTPS and the public routes. Do not replace a session-aware application host with this library-only static site. Tuilith has no hosted account or signed-in root.

Native desktop/mobile visuals, actual browser clipboard, remote font loading and pinned CI toolchain gates remain distinct from static output and controlled DOM tests. Future source revisions must refresh the original inventory from verified owning bytes, not edit hashes to hide unexpected files.
