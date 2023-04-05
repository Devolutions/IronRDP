import type {ScreenScale} from '../enums/ScreenScale';
import type {Observable} from 'rxjs';
import type {NewSessionInfo} from './NewSessionInfo';

export interface IRGUserInteraction {
    setVisibility(state: boolean);

    setScale(scale: ScreenScale);

    connect(username: string, password: string, hostname: string, gatewayAddress: string, domain: string, authToken: string): Observable<NewSessionInfo>;

    ctrlAltDel();

    metaKey();

    sessionListener: Observable<any>;
}
