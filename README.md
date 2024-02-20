# Pdrive CLI Client
This is a CLI tool that uploads files to a cloudflare R2 Bucket. See [this repo](https://github.com/TraceLTRC/personal-drive) for the server implementation.

## Configuring
The configuration file is created after the first launch of the tool, and it exists in the following directory:

Windows: `%APPDATA%\pdrive\config\default-config.toml`

Linux: `$XDG_CONFIG_HOME/pdrive/default-config.toml`

Make sure to set the API endpoint, and token to the correct values.
