import init, {
    iron_init,
    DesktopSize,
    DeviceEvent,
    InputTransaction,
    IronError,
    Session,
    SessionBuilder,
    SessionTerminationInfo,
    ClipboardTransaction,
    ClipboardContent,
} from '../../../crates/ironrdp-web/pkg/ironrdp_web';
import { preConnectionBlob, kdcProxyUrl, displayControl } from './services/ExtensionBuilders';

export default {
    init,
    iron_init,
    DesktopSize,
    DeviceEvent,
    InputTransaction,
    IronError,
    SessionBuilder,
    ClipboardTransaction,
    ClipboardContent,
    Session,
    SessionTerminationInfo,
};

export { preConnectionBlob, kdcProxyUrl, displayControl };
