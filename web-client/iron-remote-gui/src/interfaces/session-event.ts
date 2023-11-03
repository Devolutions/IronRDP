import type { IronRdpError } from '../../../../crates/ironrdp-web/pkg/ironrdp_web';
import type { SessionEventType } from '../enums/SessionEventType';

export interface SessionEvent {
	type: SessionEventType;
	data?: IronRdpError | string;
}
