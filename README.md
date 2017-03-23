#proddle

##Overview
A distributed network measurement tool.

##Logging
We are using the env_logger crate provided by rust to perform
all logging operations. To enable logging an environment
variable "RUST_LOG" is requried to be set. To execute a
binary using this framework use the command examples.

env RUST_LOG=info ./bridge
env RUST_LOG=info ./vantage -H hostname.example.com -I 1.2.3.4

##TODO
- (everywhere) change measurement to measurement_class
- (everywhere) change result to measurement
- upgrade bson to "0.5"
- bridge/vantage - add tokio_io crate to remove deprecated tokio_core items

- validate hostname and ip address on vantage
