# Third-party notices

The project source and the procedural demo artwork are licensed under MIT; see
[LICENSE](LICENSE). Dependencies retain their own licenses.

The local JavaScript runtime and historical benchmark bundle include:

- Three.js 0.187.0: MIT, copyright Three.js authors. Full notice:
  [licenses/three.txt](licenses/three.txt).
- Spark 2.4.0: MIT, copyright WORLD LABS TECHNOLOGIES, INC. Full notice:
  [licenses/spark.txt](licenses/spark.txt).

The Rust application depends on GPUI Kit 0.9.0, GPUI WebView 0.9.0 and GPUI
`gpui-pre` 0.3.11, whose package metadata declares Apache-2.0. These dependencies
are obtained by Cargo, not vendored into this repository. Lockfiles record the
full dependency graph; binary distributions must retain the notices applicable
to the dependencies included in them.

FFmpeg and Chrome are external tools, not bundled here. ProUI is a visual
reference; its commercial kit and original demo photographs are not distributed.
The two demo PNGs are original procedural illustrations reproducible with
`python3 scripts/generate-demo-assets.py`.
