# SvelteKit UI for IronRDP

The UI is used both for Tauri Desktop App and Browser App.

## Tauri

Please read the [README](../iron-tauri-client/) from `iron-tauri-client`

## Web client

Web client is built on top of [SvelteKit](https://kit.svelte.dev/). 
It's a simple wrapper around `iron-remote-gui` demonstrating how to use the API.
The core of the web client is to be found in `iron-remote-gui` folder provided as a Web Component.

### Requirements

- A remote machine ready to receive RDP connections using RemoteFX (see top-level [README](../README.md) on that matter).
- A Devolutions Gateway with network access to this machine up and running
- A token signed using the provisioner key the Devolutions Gateway is expecting

#### Devolutions Gateway setup

Web client is using a special extension to RDP protocol that is only supported in the latest Devolutions Gateway builds.

You can either:

- Download a binary prebuilt from master such as [this one](https://devolutions.sharepoint.com/:f:/s/Prereleases/Ei4GzG25BWhKtmrJiurIjDEBkd8j1VWy4fzaWR42ew4f8g?e=H3bFFM).
- Build [master](https://github.com/Devolutions/devolutions-gateway/tree/master) yourself.
  Simply [install the Rust toolchain](https://rustup.rs/) and run `cargo build --release`. Binary will be found in the `./target/release` folder.

Create a new folder somewhere on your system. For instance `/home/david/Documents/gateway-config`.
We’ll store Devolutions Gateway configuration inside this folder.

Set the `DGATEWAY_CONFIG_PATH` environment variable to this path.

PowerShell:

```pwsh
$ $Env:DGATEWAY_CONFIG_PATH = "/home/david/Documents/gateway-config"
```

bash / zsh /other bash-like shells:

```bash
$ export DGATEWAY_CONFIG_PATH=/home/david/Documents/gateway-config
```

Generate a default configuration using the Devolutions Gateway executable:

```shell
$ ./DevolutionsGateway --config-init-only # Linux / macOS
$ ./DevolutionsGateway.exe --config-init-only # Windows
```

For convenience, modify the freshly generated `gateway.json` like so:

```json
{
  // -- snip -- //
  "__debug__": {
    "disable_token_validation": true
  }
}
```

That way, you can later reuse the same token multiple times (convenient at development time).

Notice that the configuration file refer to a public (provisioner) key file called `provisioner.pem`.
We need to generate this file.

You can an RSA key pair using `openssl` by running the following commands:

```shell
$ openssl genrsa -out provisioner.key 2048
$ openssl rsa -in provisioner.key -outform PEM -pubout -out provisioner.pem
```

Where `provisioner.key` is our private key and `provisioner.pem` the public counterpart.

Congratulations, your Devolutions Gateway setup is complete.
Assuming the environment variable is properly set, you can run the executable:

```shell
$ ./DevolutionsGateway # Linux / macOS
$ ./DevolutionsGateway.exe # Windows
```

#### Token generation

The most straightforward way of generating a token if you don’t have a Rust toolchain installed is
the PowerShell package.

```pwsh
$ Install-Module -Name DevolutionsGateway
```

You can then run the following:

```pwsh
$ New-DGatewayToken -Type ASSOCIATION -PrivateKeyFile <PRIVATE KEY PATH> -DestinationHost <TARGET HOST> -ApplicationProtocol rdp
```

### Run in dev mode

First, run `npm install` in the [iron-remote-gui](../iron-remote-gui/) folder,
and then `npm install` in [iron-svelte-client](./) folder.

You can then start the dev server with either: 

- `npm run dev` - Runs only the final application.
- `npm run dev-all` - Builds WASM module and `iron-remote-gui` prior to starting the dev server.
- `npm run dev-no-wasm` - Only builds `iron-remote-gui` prior to starting the dev server.

You can build distribution files with `npm run build`.
Files are to be found in `./iron-svelte-client/build/browser`
