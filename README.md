# Foxglove WebSocket publishing in Rust

[![codecov](https://codecov.io/gh/dmweis/foxglove-ws/branch/main/graph/badge.svg)](https://codecov.io/gh/dmweis/foxglove-ws)
[![Rust](https://github.com/dmweis/foxglove-ws/workflows/Rust/badge.svg)](https://github.com/dmweis/foxglove-ws/actions)
[![Private docs](https://github.com/dmweis/foxglove-ws/workflows/Deploy%20Docs%20to%20GitHub%20Pages/badge.svg)](https://davidweis.dev/foxglove-ws/foxglove_ws/index.html)

This library provides means to publish messages to the amazing Foxglove UI in
Rust. It implements part of the Foxglove WebSocket protocol described in
<https://github.com/foxglove/ws-protocol>.

## Example

Call

```shell
cargo run --release --example string
```

to start an example application that publishes ROS 1 string data -- both
latching and non-latching.
