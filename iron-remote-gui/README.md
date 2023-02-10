# Iron Remote Gui

This is the core of the web client. Written with Svelte and build as Web-component.

## Development

Make you modification in the source code then use [iron-svelte-client](../iron-svelte-client) to test.

## Build

Run `npm run build`

## Usage

As Devolutions team member, you can import the web-component as an Artifactory package. Just run `npm install @devolutions/iron-remote-gui`.
Otherwise you can make an npm install targeting the /dist folder.

Import the `iron-remote-gui.umd.cjs` from node_modules folder.

Then use the html tag `<iron-remote-gui/>` in your page.

In your code add a listener on the `ready` event on the iron-remote-gui html element.
Get `evt.detail.irgUserInteraction` from the promise. This object is of type `UserInteractionService`.
After that you need to call, at least, `connect` from the `UserInteractionService`.

## Limitations

For now, we didn't make the enums used by some method directly available (I didn't find the good way to export them directly with the component.).
You need to recreate them on your application for now (it will be improve in futur version);

Also even if the connection to RDP work there is still a lot of improvement to do. 
At now you can expect, mouse movement and click (4 buttons) - no scroll, Keyboard for at least the standard. Windows and CTRL+ALT+DEL can be called by method on UserInteractionService. 
Lock keys (like caps lock), have a partial support. 
Other advanced functionalities (sharing / copy past...) are not implemented yet.

## IronRemoteGui parameters

You can add some parameters for default initialisation on the component <iron-remote-gui />.
> Note that due to a limitation of the framework all parameters need to be in minuscule
- `scale`: The scaling behavior of the distant screen. Can be 'fit', 'real' or 'full'. Default is 'real';
- `targetplatform`: Can be 'web' or 'native'. Default is 'web'.
- `verbose`: Show logs from iron-remote-gui. 'true' or 'false'. Default is 'false'.
- `debugwasm`: Show debug info from web assembly. Can be "OFF", "ERROR", "WARN", "INFO", "DEBUG", "TRACE". Default is 'OFF'.
- `flexcentre`: Helper to force iron-remote-gui a flex and centering the content automatically. Otherwise you need to manage manually. Default is 'true'.

## UserInteractionService methods
>`connect(username: string, password: string, host: string, authtoken: string): Observable<NewSessionInfo>`
>
> username and password are the credentials use to connect on the remote. Host is the jet server of the gateway. Authtoken is the gateway token.

> `ctrlAltDel()`
> 
> Send the ctrl+alt+del key to server.

> `metaKey()`
> 
> Send the metakey to server (like windows key).

> `setVisibility(value: bool)`
> 
> Show or Hide canvas.

> `setScale(scale: ScreenScale)`
> 
> Set the scale behavior of the canvas. (please take a look on the [ScreenScale](src/services/user-interaction-service.ts) enum)

