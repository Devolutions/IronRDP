# IronRDP RDPECAM native backends

Native camera capture backends for [`ironrdp-rdpecam`](../ironrdp-rdpecam).

On Windows, `MediaFoundationCamera` implements `CameraBackend` with a Media Foundation source reader
(`IMFSourceReader`) opened on a video capture device:

- `enumerate_cameras` lists video capture devices. Each device is identified by its symbolic link,
  which stays stable across reboots and matches the `KSCATEGORY_VIDEO_CAMERA` interface path used by
  the ActiveX camera collection.
- The device's native media types are mapped to the MS-RDPECAM formats (H.264, MJPEG, YUY2, NV12,
  I420, RGB24, RGB32). When the camera produces a compressed format, only compressed media types are
  advertised so that raw frames do not saturate the connection.
  Compressed samples are passed through as the camera produces them, and H.264 key frames get the
  sequence header prepended when the camera keeps it out of band.
- Version 2 device properties are backed by `IAMCameraControl` and `IAMVideoProcAmp`.
- `CameraWatcher` reports device arrival and removal through configuration manager notifications.

All Media Foundation objects live on one worker thread per activated device, initialized for the
multithreaded apartment, so the backend can be driven from any thread.

Other platforms currently have no backend.
