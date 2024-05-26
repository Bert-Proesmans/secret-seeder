#!/bin/sh

# Use this file to properly load Rust-Analyzer within the context of 
# a Rust project workspace using Remote Connections over SSH.
#
# eg;
# "rust-analyzer.server.path": "/path/to/this/file.sh"

# There is no way to pass a working directory into the script invocation,
# so the script itself will switch to the proper directory before running
# rust-analyzer.
SCRIPT_DIR=$(dirname "$(readlink -f "$0")")
cd "$SCRIPT_DIR" || exit

# Load direnv environment
eval "$(direnv export bash)"
# Start rust-analyzer
exec rust-analyzer "$@"