import type { SessionEventType } from '../enums/SessionEventType';

export interface SessionEvent {
	type: SessionEventType;
	data?: unknown;
}
