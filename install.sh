#!/bin/bash
set -e

# SSH Manager Installation Script
# Installs the latest release of sshm

REPO_OWNER="titonio"
REPO_NAME="ssh-manager"
BIN_NAME="sshm"

echo "Installing $REPO_NAME..."

# Detect platform
if [ "$(uname -s)" = "Linux" ]; then
    PLATFORM="x86_64-unknown-linux-musl"
elif [ "$(uname -s)" = "Darwin" ]; then
    if [ "$(uname -m)" = "arm64" ]; then
        PLATFORM="aarch64-apple-darwin"
    else
        PLATFORM="x86_64-apple-darwin"
    fi
elif [ "$(uname -s)" = "Windows" ]; then
    PLATFORM="x86_64-pc-windows-msvc"
    echo "Windows support coming soon!"
    exit 1
else
    echo "Unsupported platform: $(uname -s)"
    exit 1
fi

echo "Platform detected: $PLATFORM"

# Get latest release
LATEST_RELEASE=$(curl -sL "https://api.github.com/repos/$REPO_OWNER/$REPO_NAME/releases/latest")
VERSION=$(echo "$LATEST_RELEASE" | grep -o '"tag_name": "[^"]*' | cut -d'"' -f4)

if [ -z "$VERSION" ]; then
    echo "Failed to get latest release version"
    exit 1
fi

echo "Latest version: $VERSION"

# Get download URL
DOWNLOAD_URL=$(echo "$LATEST_RELEASE" | grep -o "\"browser_download_url\": \"[^\"]*${PLATFORM}[^\"]*\"" | cut -d'"' -f4)

if [ -z "$DOWNLOAD_URL" ]; then
    echo "No release found for platform: $PLATFORM"
    echo "Available assets:"
    echo "$LATEST_RELEASE" | grep -o '"browser_download_url": "[^"]*"' || true
    exit 1
fi

echo "Downloading from: $DOWNLOAD_URL"

# Create temp directory
TEMP_DIR=$(mktemp -d)
trap "rm -rf $TEMP_DIR" EXIT

# Download and extract
curl -sL "$DOWNLOAD_URL" -o "$TEMP_DIR/$BIN_NAME.tar.gz"

echo "Extracting..."
tar -xzf "$TEMP_DIR/$BIN_NAME.tar.gz" -C "$TEMP_DIR"

# Install binary
if [ -w "/usr/local/bin" ]; then
    sudo mv "$TEMP_DIR/$BIN_NAME" "/usr/local/bin/$BIN_NAME"
    sudo chmod +x "/usr/local/bin/$BIN_NAME"
else
    # Try user installation
    if [ -d "$HOME/.local/bin" ]; then
        mkdir -p "$HOME/.local/bin"
        mv "$TEMP_DIR/$BIN_NAME" "$HOME/.local/bin/$BIN_NAME"
        chmod +x "$HOME/.local/bin/$BIN_NAME"
        echo "Installed to $HOME/.local/bin/$BIN_NAME"
        echo "Please add \$HOME/.local/bin to your PATH"
    else
        mkdir -p "$HOME/.local/bin"
        mv "$TEMP_DIR/$BIN_NAME" "$HOME/.local/bin/$BIN_NAME"
        chmod +x "$HOME/.local/bin/$BIN_NAME"
        echo "Installed to $HOME/.local/bin/$BIN_NAME"
        echo "Please add \$HOME/.local/bin to your PATH"
    fi
fi

echo ""
echo "Installation complete!"
echo "Run '$BIN_NAME' to start the application"

# Install shell completions
echo ""
echo "Installing shell completions..."

# Get the shell we're running in
CURRENT_SHELL="${SHELL##*/}"

case "$CURRENT_SHELL" in
    bash)
        COMPLETION_FILE="$HOME/.bash_completion.d/$BIN_NAME"
        mkdir -p "$HOME/.bash_completion.d"
        $BIN_NAME completions bash > "$COMPLETION_FILE" 2>/dev/null || true
        echo "Bash completions installed to $COMPLETION_FILE"
        echo "Please run 'source $COMPLETION_FILE' or restart your shell"
        ;;
    zsh)
        COMPLETION_FILE="$HOME/.zsh/_$BIN_NAME"
        mkdir -p "$HOME/.zsh"
        $BIN_NAME completions zsh > "$COMPLETION_FILE" 2>/dev/null || true
        echo "Zsh completions installed to $COMPLETION_FILE"
        echo "Add 'fpath=(~/.zsh \$fpath)' and 'autoload -U compinit; compinit' to your .zshrc"
        ;;
    fish)
        COMPLETION_FILE="$HOME/.config/fish/completions/$BIN_NAME.fish"
        mkdir -p "$HOME/.config/fish/completions"
        $BIN_NAME completions fish > "$COMPLETION_FILE" 2>/dev/null || true
        echo "Fish completions installed to $COMPLETION_FILE"
        ;;
    *)
        echo "Shell completions for $CURRENT_SHELL not automatically installed."
        echo "To install manually, run:"
        echo "  $BIN_NAME completions bash/zsh/fish > ~/.config/completions/$BIN_NAME"
        ;;
esac
