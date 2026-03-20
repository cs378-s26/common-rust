#!/bin/sh
cargo fmt --all &&
cargo clippy --target x86_64-unknown-none -Zbuild-std=core,alloc -Zpanic-abort-tests &&
cargo clippy --target aarch64-unknown-none -Zbuild-std=core,alloc -Zpanic-abort-tests &&
cargo clippy --tests --target x86_64-unknown-none -Zbuild-std=core,alloc -Zpanic-abort-tests &&
cargo clippy --tests --target aarch64-unknown-none -Zbuild-std=core,alloc -Zpanic-abort-tests &&
cargo buildtool test &&
echo "commit ready to go!"
