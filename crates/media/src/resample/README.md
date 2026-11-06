# Audio conversion

- `AudioResampler`: explicit continuous-span rate/layout/sample-format conversion.
  Admits output rates of 8–192 kHz and up to 24× expansion. Keeps the source origin and counts output samples in integer time; drain `flush`.
  `convert` returns bounded frames and drains buffered output without finalizing
  the filter. A discontinuity requires a new converter. `AudioEncoder` retains source rate,
  converts sample format, and uses a bounded FFmpeg FIFO for AAC/Opus frame sizes.
  FFmpeg owns codec delay and final padding.
