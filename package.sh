#!/bin/bash
# package.sh - InternalVoice Unix Installation & Wrapper Script

set -e

# --- Configuration ---
BINARY_NAME="internalvoice"
TARGET_PATH="./target/release/$BINARY_NAME"
WRAPPER_NAME="iv"

# --- Helper Functions ---

check_rust() {
    if ! command -v rustc &> /dev/null; then
        echo "Error: Rust compiler (rustc) not found."
        read -p "Would you like to install Rust now via rustup? (y/n) " -n 1 -r
        echo
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            echo "Installing Rust..."
            curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
            source "$HOME/.cargo/env"
        else
            echo "Rust is required to build InternalVoice. Please install it and try again."
            exit 1
        fi
    fi
}

build_app() {
    echo "Building $BINARY_NAME in release mode..."
    cargo build --release
}

run_setup() {
    echo "Running configuration wizard..."
    $TARGET_PATH --setup
}

create_wrapper() {
    echo "Creating '$WRAPPER_NAME' command wrapper..."
    cat <<EOF > "$WRAPPER_NAME"
#!/bin/bash
# InternalVoice Wrapper
DIR="\$( cd "\$( dirname "\${BASH_SOURCE[0]}" )" && pwd )"
cd "\$DIR"
./target/release/$BINARY_NAME "\$@"
EOF
    chmod +x "$WRAPPER_NAME"
    echo "  [✓] Wrapper created: ./$WRAPPER_NAME"
}

# --- Main Execution ---

echo "── InternalVoice Packaging & Setup ──"
echo

check_rust
build_app
run_setup
create_wrapper

echo
echo "Successfully structured packaging for InternalVoice."
echo "You can now run the program using: ./$WRAPPER_NAME"
