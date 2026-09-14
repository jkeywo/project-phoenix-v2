# WebKit composite measured HRTF responses

`Composite.wav` is unchanged from WebKit commit
`644c76832a0675410aebf50f9bc33088c40a82b3`:
https://github.com/WebKit/WebKit/blob/644c76832a0675410aebf50f9bc33088c40a82b3/Source/WebCore/platform/audio/resources/Composite.wav

SHA-256: `a25fc99a6e875f659aaa9d107c791aeff250030e0d7f16458baf3a00943394e3`.

The sample data is public domain. Chris Rogers derived the composite responses
for WebKit by averaging measurements from the IRCAM Listen HRTF Database. The
public-domain grant for these samples is documented in Mozilla's corresponding
data declaration:
https://github.com/mozilla/gecko-dev/blob/master/dom/media/webaudio/blink/IRC_Composite_C_R0195-incl.cpp

Only the sample data is vendored here; no Mozilla or WebKit implementation code
is copied. These are measured spatial filters, not new game sounds or speech.

The WAV contains 240 stereo impulse responses at 44,100 Hz, 256 frames each.
Database azimuth increases toward the listener's left from forward in 15-degree
steps, opposite the Web Audio panner angle (WebKit's `HRTFPanner.cpp` applies
that sign conversion before lookup). For each azimuth,
elevations are ordered 0, 15, 30, 45, 60, 75, 90, -45, -30, -15 degrees. That
layout is documented by WebKit's `HRTFElevation.cpp`. Phoenix interpolates the
measured coefficients. Convolution runs at least at the measured 44,100 Hz rate:
lower-rate source PCM is upsampled first, rather than decimating and aliasing the
measured filter. Higher-rate sources use interpolated filters at their rate.
Authored gain, distance and the endpoint mix apply separately.
