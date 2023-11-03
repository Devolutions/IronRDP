# SvelteKit UI for IronRDP

Web-based frontend using [`SvelteKit`](https://kit.svelte.dev/) and [`Material`](https://material.io) frameworks.
This is a simple wrapper around the `iron-remote-gui` Web Component demonstrating how to use the API.

## Requirements

-   A Devolutions Gateway with network access to this machine up and running
-   A token signed using the provisioner key the Devolutions Gateway is expecting

### Devolutions Gateway setup

Web client is using a special extension to RDP protocol.
This extension is available starting Devolutions Gateway v2023.1.1.
However, this version not yet officially published.

Therefore, you need to either:

-   Download a binary prebuilt from master such as [this one](https://devolutions.sharepoint.com/:f:/s/Prereleases/En3Y3T3OIuFFpYknTZYZfIYBXo_OpCubXBKd8wpjZ7Qrtg?e=MBVz53).
-   Build [master](https://github.com/Devolutions/devolutions-gateway/tree/master) yourself.
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

Notice that the configuration file refers to a public (provisioner) key file called `provisioner.pem`.
We need to generate this file.

You can generate an RSA key pair using `openssl` by running the following commands:

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

### Token generation

The most straightforward way of generating a token if you don’t have a Rust toolchain installed is
the PowerShell package.

```pwsh
$ Install-Module -Name DevolutionsGateway
```

You can then run the following:

```pwsh
$ New-DGatewayToken -Type ASSOCIATION -PrivateKeyFile <PRIVATE KEY PATH> -DestinationHost <TARGET HOST> -ApplicationProtocol rdp
```

## Run in dev mode

First, run `npm install` in the [iron-remote-gui](../iron-remote-gui/) folder,
and then `npm install` in [iron-svelte-client](./) folder.

You can then start the dev server with either:

-   `npm run dev` - Runs only the final application.
-   `npm run dev-all` - Builds WASM module and `iron-remote-gui` prior to starting the dev server.
-   `npm run dev-no-wasm` - Only builds `iron-remote-gui` prior to starting the dev server.

You can build distribution files with `npm run build`.
Files are to be found in `./iron-svelte-client/build/browser`
