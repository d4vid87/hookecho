# On-demand HLS decoder

`hls-1.7.3.min.js` is the unmodified `dist/hls.min.js` from the `hls.js@1.7.3` npm package ([upstream](https://github.com/video-dev/hls.js)). SHA-256: `a12e7ee1cd64a69dcdb314157e45dafcba705bfb0b1440b7935cb265d374423e`. Its Apache-2.0 license is in `hls-1.7.3.LICENSE`.

`web/video-player.js` loads this file only when a user opens an HLS camera. It is not part of the radar startup bundle or offline shell.
