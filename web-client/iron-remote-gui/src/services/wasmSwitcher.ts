import * as rdp from '../ironrdp/pkg/ironrdp_web';
import * as vnc from '../ironvnc/pkg/ironvnc_web';
import { loggingService } from './logging.service';

export type RemoteConnectionError = rdp.IronRdpError | vnc.IronRdpError;
export type Session = rdp.Session | vnc.Session;
export type SessionTerminationInfo = rdp.SessionTerminationInfo | vnc.SessionTerminationInfo;
export type DeviceEvent = rdp.DeviceEvent | vnc.DeviceEvent;
export default async function init(type: 'rdp' | 'vnc'): Promise<typeof rdp | typeof vnc> {
    if (type === 'rdp') {
        loggingService.info('Initializing RDP');
        await rdp.default();
        return rdp;
    } else {
        loggingService.info('Initializing VNC');
        await vnc.default();
        return vnc;
    }
}
