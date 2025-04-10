# SvelteKit UI for IronRDP

Web-based frontend using [`SvelteKit`](https://kit.svelte.dev/) and [`Material`](https://material.io) frameworks.
This is a simple wrapper around the `iron-remote-desktop` Web Component demonstrating how to use the API.

Note that this demonstration client is not intended to be used in production as-is.
Devolutions is shipping well-integrated, production-ready IronRDP web clients as part of:

- [Devolutions Gateway](https://github.com/Devolutions/devolutions-gateway/)’s [Standalone Web Interface](https://github.com/Devolutions/devolutions-gateway/tree/master/webapp) which is a completely free product (shipped since v2024.1.0).
- [Devolutions Server](https://devolutions.net/server/), our self-hosted credential manager.
- [Devolutions Hub](https://devolutions.net/password-hub/), our cloud-based credential manager.

## Requirements

- A Devolutions Gateway with network access to this machine up and running
- A token signed using the provisioner key configured on the Devolutions Gateway

### Devolutions Gateway setup

The IronRDP web client is relying on an extension to the RDP protocol ("RDCleanPath").
This enables us to avoid the redundant TLS layer, or "TLS-in-TLS" problem found in other RDP web clients.
This redundant TLS layer is typically required to circumvent the restriction imposed by web browsers.
Indeed, it’s not possible to open a plain TCP socket using the API provided by web browsers.
Instead, we need a middleware service to unpack the WebSocket payload and forward it over a plain TCP transport.
Other web clients are using a Secure WebSocket transport (WebSocket over TLS) to communicate with the middleware,
and inside this secure transport another protocol-level, extra TLS transport is opened.
With our extension, the middleware service inspects the RDP handshake and perform the TLS upgrade on its end, removing the need for the redundant client-side TLS encryption.
The extension is supported by the [Devolutions Gateway](https://github.com/Devolutions/devolutions-gateway/) (v2023.1.1 and later).

You need to install and configure it in order to use the web client.
You can follow the instructions found on the dedicated repository.

You will need to generate a key pair, that we call the "provisioner" key pair.
You can generate an RSA key pair using `openssl` by running the following commands:

```shell
$ openssl genrsa -out provisioner.key 2048
$ openssl rsa -in provisioner.key -outform PEM -pubout -out provisioner.pem
```

Where `provisioner.key` is the private part and `provisioner.pem` the public counterpart.
The public one must be installed on the Devolutions Gateway.

Once installed, you can optionally modify the `gateway.json` config file to add the following debug option:

```json
{
  // -- snip -- //
  "__debug__": {
    "disable_token_validation": true
  }
}
```

That way, you can later reuse the same token multiple times (convenient at development time).

Make sure to start or restart the service before proceeding further.

### Token generation

### Automatic Token Generation

**Prerequisites:**  
Ensure the Rust toolchain is installed and available on your system.

#### Steps:

1. **Locate the `tokengen` Project:**  
   Navigate to the root directory of the `tokengen` project under the [Devolutions Gateway repository](https://github.com/Devolutions/devolutions-gateway/tree/master/tools/tokengen).

2. **Set the Configuration Path:**  
   Define the environment variable `DGATEWAY_CONFIG_PATH` to the directory containing your `gateway.json` file.
   Also ensure the `ProvisionerPrivateKeyFile` config key is properly set in the `gateway.json` file.

3. **Run the tokengen server:**  
   In the root of the `tokengen` project, execute the following command:

   ```sh
   cargo run -- server
   ```

4. **Configure the Environment Variable for Vite:**  
   Either update the `.env` file or manually set the following environment variable:

   ```sh
   VITE_IRON_TOKEN_SERVER_URL="http://localhost:8080"
   ```

   Ensure that Vite correctly detects this variable.

Once these steps are completed, token generation will be fully automated, and the next section can be ignored.

#### Manual token generation

The most straightforward way of generating a token if you don’t have a Rust toolchain installed is
the PowerShell package.

```pwsh
$ Install-Module -Name DevolutionsGateway
```

You can then run the following:

```pwsh
$ New-DGatewayToken -Type ASSOCIATION -PrivateKeyFile <PRIVATE KEY PATH> -DestinationHost <TARGET HOST> -ApplicationProtocol rdp
```

If you have a Rust toolchain available, you can use the [`tokengen`][tokengen] tool found in Devolutions Gateway repository.

[tokengen]: https://github.com/Devolutions/devolutions-gateway/tree/master/tools/tokengen

## Run in development mode

First, run `npm install` in [iron-remote-desktop](../iron-remote-desktop) and [iron-remote-desktop-rdp](../iron-remote-desktop-rdp) folders, and then `npm install` in [iron-svelte-client](./) folder.

You can then start the dev server with either:

- `npm run dev` - Runs only the final application.
- `npm run dev-all` - Builds WASM module and `iron-remote-desktop` prior to starting the dev server.
- `npm run dev-no-wasm` - Only builds `iron-remote-desktop` prior to starting the dev server.

You can build distribution files with `npm run build`.
Files are to be found in `./iron-svelte-client/build/browser`.

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP
