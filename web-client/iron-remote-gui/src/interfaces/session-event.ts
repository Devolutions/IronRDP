import type {SessionEventType} from '../enums/SessionEventType';
import type { IronRdpError } from '../../../../crates/ironrdp-web/pkg/ironrdp_web';

export interface SessionEvent {
    type: SessionEventType,
    data?: IronRdpError | string
}
