# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.10.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.9.1...ironrdp-v0.10.0)] - 2025-04-11

### <!-- 1 -->Features

- Add no_audio_playback flag to Config struct ([9f0edcc4c9](https://github.com/Devolutions/IronRDP/commit/9f0edcc4c9c49d59cc10de37f920aae073e3dd8a)) 

  Enable audio playback on the client.

### <!-- 7 -->Build

- Bump the patch group across 1 directory with 2 updates (#731) ([ba488f956c](https://github.com/Devolutions/IronRDP/commit/ba488f956c13538b37f1d3444afbbb2915ea37d6)) 

  Bumps the patch group with 2 updates in the / directory:
  [image](https://github.com/image-rs/image) and
  [clap](https://github.com/clap-rs/clap).
  
  Updates `image` from 0.25.5 to 0.25.6
  <details>
  <summary>Changelog</summary>
  <p><em>Sourced from <a
  href="https://github.com/image-rs/image/blob/main/CHANGES.md">image's
  changelog</a>.</em></p>
  <blockquote>
  <h3>Version 0.25.6</h3>
  <p>Features:</p>
  <ul>
  <li>Improved format detection (<a
  href="https://redirect.github.com/image-rs/image/pull/2418">#2418</a>)</li>
  <li>Implement writing ICC profiles for JPEG and PNG images (<a
  href="https://redirect.github.com/image-rs/image/pull/2389">#2389</a>)</li>
  </ul>
  <p>Bug fixes:</p>
  <ul>
  <li>JPEG encoding bugfix (<a
  href="https://redirect.github.com/image-rs/image/pull/2387">#2387</a>)</li>
  <li>Expanded ICO format detection (<a
  href="https://redirect.github.com/image-rs/image/pull/2434">#2434</a>)</li>
  <li>Fixed EXR bug with NaNs (<a
  href="https://redirect.github.com/image-rs/image/pull/2381">#2381</a>)</li>
  <li>Various documentation improvements</li>
  </ul>
  </blockquote>
  </details>
  <details>
  <summary>Commits</summary>
  <ul>
  <li><a
  href="https://github.com/image-rs/image/commit/f337e27aadaae8b86484429bc6020fef8a019c95"><code>f337e27</code></a>
  Release 0.25.6 (<a
  href="https://redirect.github.com/image-rs/image/issues/2441">#2441</a>)</li>
  <li><a
  href="https://github.com/image-rs/image/commit/0166f687e9276cec9081a72488ba1f0c9bd88608"><code>0166f68</code></a>
  CI: add num-traits to public (<a
  href="https://redirect.github.com/image-rs/image/issues/2446">#2446</a>)</li>
  <li><a
  href="https://github.com/image-rs/image/commit/ca9e2dceb436a8c5a8202797cb9e8a1573eba35e"><code>ca9e2dc</code></a>
  add links to readme (<a
  href="https://redirect.github.com/image-rs/image/issues/2437">#2437</a>)</li>
  <li><a
  href="https://github.com/image-rs/image/commit/95be33928ea284a4621bae7e06abb17025a66df4"><code>95be339</code></a>
  Making clippy happy (<a
  href="https://redirect.github.com/image-rs/image/issues/2439">#2439</a>)</li>
  <li><a
  href="https://github.com/image-rs/image/commit/c62d3ace614155ac46c95b85b1ec86db337d15c0"><code>c62d3ac</code></a>
  Detect image/vnd.microsoft.icon mime types as ImageFormat::Ico (<a
  href="https://redirect.github.com/image-rs/image/issues/2434">#2434</a>)</li>
  <li><a
  href="https://github.com/image-rs/image/commit/85f2412d552ddd2f576e16d023fd352589f4c605"><code>85f2412</code></a>
  Fix missing spaces in JpegDecoder error message (<a
  href="https://redirect.github.com/image-rs/image/issues/2433">#2433</a>)</li>
  <li><a
  href="https://github.com/image-rs/image/commit/b22ba14127749ce821b03119a492c776fc1846d4"><code>b22ba14</code></a>
  Remove limits when parsing JPEG metadata (<a
  href="https://redirect.github.com/image-rs/image/issues/2429">#2429</a>)</li>
  <li><a
  href="https://github.com/image-rs/image/commit/4ef6f1505cb0b320c530d8b0a029d0cfa4b13b14"><code>4ef6f15</code></a>
  Fix unbalanced backticks in doc comments (<a
  href="https://redirect.github.com/image-rs/image/issues/2427">#2427</a>)</li>
  <li><a
  href="https://github.com/image-rs/image/commit/d4054385a1b6071bfa34e045f4b598d31d68f41f"><code>d405438</code></a>
  Reduce typo count (<a
  href="https://redirect.github.com/image-rs/image/issues/2426">#2426</a>)</li>
  <li><a
  href="https://github.com/image-rs/image/commit/68159de1c1f57653c0ce93422921b32633b6bd45"><code>68159de</code></a>
  Update resize and blurs doc (<a
  href="https://redirect.github.com/image-rs/image/issues/2424">#2424</a>)</li>
  <li>Additional commits viewable in <a
  href="https://github.com/image-rs/image/compare/v0.25.5...v0.25.6">compare
  view</a></li>
  </ul>
  </details>
  <br />
  
  Updates `clap` from 4.5.32 to 4.5.34
  <details>
  <summary>Release notes</summary>
  <p><em>Sourced from <a
  href="https://github.com/clap-rs/clap/releases">clap's
  releases</a>.</em></p>
  <blockquote>
  <h2>v4.5.34</h2>
  <h2>[4.5.34] - 2025-03-27</h2>
  <h3>Fixes</h3>
  <ul>
  <li><em>(help)</em> Don't add extra blank lines with
  <code>flatten_help(true)</code> and subcommands without arguments</li>
  </ul>
  <h2>v4.5.33</h2>
  <h2>[4.5.33] - 2025-03-26</h2>
  <h3>Fixes</h3>
  <ul>
  <li><em>(error)</em> When showing the usage of a suggestion for an
  unknown argument, don't show the group</li>
  </ul>
  </blockquote>
  </details>
  <details>
  <summary>Changelog</summary>
  <p><em>Sourced from <a
  href="https://github.com/clap-rs/clap/blob/master/CHANGELOG.md">clap's
  changelog</a>.</em></p>
  <blockquote>
  <h2>[4.5.34] - 2025-03-27</h2>
  <h3>Fixes</h3>
  <ul>
  <li><em>(help)</em> Don't add extra blank lines with
  <code>flatten_help(true)</code> and subcommands without arguments</li>
  </ul>
  <h2>[4.5.33] - 2025-03-26</h2>
  <h3>Fixes</h3>
  <ul>
  <li><em>(error)</em> When showing the usage of a suggestion for an
  unknown argument, don't show the group</li>
  </ul>
  </blockquote>
  </details>
  <details>
  <summary>Commits</summary>
  <ul>
  <li><a
  href="https://github.com/clap-rs/clap/commit/5d2cdac3e6a7aa5fc720f911a2a5a7671e610758"><code>5d2cdac</code></a>
  chore: Release</li>
  <li><a
  href="https://github.com/clap-rs/clap/commit/f1c10ebe58f888cf96b48aeb8c4b0b6d6cbc6e6f"><code>f1c10eb</code></a>
  docs: Update changelog</li>
  <li><a
  href="https://github.com/clap-rs/clap/commit/a4d1a7fe2b9dc4b52fccb15515e2931291217059"><code>a4d1a7f</code></a>
  chore(ci): Take a break from template updates</li>
  <li><a
  href="https://github.com/clap-rs/clap/commit/e95ed396c427febc684f4a0995fcbd3a025e6a37"><code>e95ed39</code></a>
  Merge pull request <a
  href="https://redirect.github.com/clap-rs/clap/issues/5775">#5775</a>
  from vivienm/master</li>
  <li><a
  href="https://github.com/clap-rs/clap/commit/18f8d4c3f5e0e2fd967e2342c4ccb030da241fe8"><code>18f8d4c</code></a>
  chore(deps): Update Rust Stable to v1.82 (<a
  href="https://redirect.github.com/clap-rs/clap/issues/5788">#5788</a>)</li>
  <li><a
  href="https://github.com/clap-rs/clap/commit/f35d8e09fbc8f72033518423d7102faa1fd50646"><code>f35d8e0</code></a>
  Merge pull request <a
  href="https://redirect.github.com/clap-rs/clap/issues/5787">#5787</a>
  from epage/template</li>
  <li><a
  href="https://github.com/clap-rs/clap/commit/1389d7d689f2730c61222d261401c7331a39ceae"><code>1389d7d</code></a>
  chore: Update from '_rust/main' template</li>
  <li><a
  href="https://github.com/clap-rs/clap/commit/dbc9faa79d67ab86cbe68da68b2cd93a0335661a"><code>dbc9faa</code></a>
  chore(ci): Initialize git for template update</li>
  <li><a
  href="https://github.com/clap-rs/clap/commit/3dac2f36833e08f6cac85b03e5907ca3dec03c4c"><code>3dac2f3</code></a>
  chore(ci): Get history for template update</li>
  <li><a
  href="https://github.com/clap-rs/clap/commit/e1f77dacf108a8cfdbe8bdff3de36bdfa3bcf50d"><code>e1f77da</code></a>
  chore(ci): Fix branch for template update</li>
  <li>Additional commits viewable in <a
  href="https://github.com/clap-rs/clap/compare/clap_complete-v4.5.32...clap_complete-v4.5.34">compare
  view</a></li>
  </ul>
  </details>
  <br />
  
  
  Dependabot will resolve any conflicts with this PR as long as you don't
  alter it yourself. You can also trigger a rebase manually by commenting
  `@dependabot rebase`.

### Refactor

- [**breaking**] Drop support for pixelOrder ([db6f4cdb7f](https://github.com/Devolutions/IronRDP/commit/db6f4cdb7f379713979b930e8e1fa1a813ebecc4)) 

  Dealing with multiple formats is sufficiently annoying, there isn't much
  need for awkward image layout. This was done for efficiency reason for
  bitmap encoding, but bitmap is really inefficient anyway and very few
  servers will actually provide bottom to top images (except with GL/GPU
  textures, but this is not in scope yet).

- [**breaking**] Use bytes, allowing shareable bitmap data ([3c43fdda76](https://github.com/Devolutions/IronRDP/commit/3c43fdda76f4ef6413db4010471364d6b1be2798)) 

- [**breaking**] Rename left/top -> x/y ([229070a435](https://github.com/Devolutions/IronRDP/commit/229070a43554927a01541052a819fe3fcd32a913)) 

  This is more idiomatic, and thus less confusing.



## [[0.9.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.9.0...ironrdp-v0.9.1)] - 2025-03-13

### <!-- 6 -->Documentation

- Fix documentation build (#700) ([0705840aa5](https://github.com/Devolutions/IronRDP/commit/0705840aa51bc920e76f0cf1fce06b29733c6e2d)) 

## [[0.9.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.8.0...ironrdp-v0.9.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu



## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.7.4...ironrdp-v0.8.0)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 

## [[0.7.4](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.7.3...ironrdp-v0.7.4)] - 2025-01-28

### Build

- Update dependencies

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 

- Extend server example to demonstrate Opus audio codec support (#643) ([fa353765af](https://github.com/Devolutions/IronRDP/commit/fa353765af016734c07e31fff44d19dabfdd4199)) 


## [[0.7.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.7.2...ironrdp-v0.7.3)] - 2024-12-16

### <!-- 6 -->Documentation

- Inline documentation for re-exported items (#619) ([cff5c1a59c](https://github.com/Devolutions/IronRDP/commit/cff5c1a59cdc2da73cabcb675fcf2d85dc81fd68)) 



## [[0.7.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.7.1...ironrdp-v0.7.2)] - 2024-12-15

### <!-- 6 -->Documentation

- Fix server example ([#616](https://github.com/Devolutions/IronRDP/pull/616)) ([02c6fd5dfe](https://github.com/Devolutions/IronRDP/commit/02c6fd5dfe142b7cc6f15cb17292504657818498)) 

  The rt-multi-thread feature of tokio is not enabled when compiling the
  example alone (without feature unification from other crates of the
  workspace).



## [[0.7.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-v0.7.0...ironrdp-v0.7.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 

