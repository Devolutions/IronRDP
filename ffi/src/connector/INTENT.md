**Compatibility sound channel**: the silent RDPSND channel a connector offers alongside RDPDR, because a Windows server does not communicate on RDPDR unless the client offers RDPSND (MS-RDPEFS Appendix A, note 1 on section 2.1).
It discards all audio it receives.
*Avoid:* "dummy channel", "noop channel".

<a id="playback-request"></a>
## Audio playback request

When the application chose playback on the server or no audio, the connector follows that choice.
Otherwise, the connector asks the server for audio only when a playback channel other than the compatibility sound channel is attached, even if the application chose playback on this computer.
Asking with only the compatibility sound channel would route the audio away from the server to a channel that discards it.

<a id="attachment-order"></a>
## Attachment order

The order in which drives, smartcard and audio are attached does not change which channels are offered or whether audio is requested.

<a id="drive-name-collisions"></a>
## Drive name collisions

A redirected drive is rejected when its name would reach the server equal to the name of a drive already selected.
The comparison may reject names the server could tell apart, and never accepts names the server would confuse.
