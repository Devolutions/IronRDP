import type { ScreenScale } from '../enums/ScreenScale';
import type { Observable } from 'rxjs';
import type { NewSessionInfo } from './NewSessionInfo';
import type { SessionEvent } from './session-event';

export interface UserInteraction {
	setVisibility(state: boolean);

	setScale(scale: ScreenScale);

	connect(
		username,
		password: string,
		destination: string,
		proxyAddress: string,
		serverDomain: string,
		authToken: string
	): Observable<NewSessionInfo>;

	ctrlAltDel();

	metaKey();

	shutdown();

	sessionListener: Observable<SessionEvent>;
}
