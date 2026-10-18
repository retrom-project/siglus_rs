# Retrom WebGL compatibility patch

Base: crates.io wgpu-hal 0.21.1, with its original MIT and Apache-2.0 licenses and Cargo source metadata retained.

The GLES ResetFramebuffer command detached all compile-time color attachment slots. Browsers exposing fewer attachments than that maximum returned INVALID_ENUM for the out-of-range detach operations. Cache the already queried device limit in AdapterShared and detach only supported slots. No error is suppressed and the device capability limit is unchanged.

Regression evidence comes from the standalone browser compatibility experiment on a WebGL2 device exposing six color attachments.
