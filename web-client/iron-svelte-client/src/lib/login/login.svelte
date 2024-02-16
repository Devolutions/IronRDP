<script lang="ts">
    import { currentSession, userInteractionService } from '../../services/session.service';
    import { catchError, filter } from 'rxjs/operators';
    import type { UserInteraction, NewSessionInfo } from '../../../static/iron-remote-gui';
    import { of } from 'rxjs';
    import { toast } from '$lib/messages/message-store';
    import { showLogin } from '$lib/login/login-store';
    import type { DesktopSize } from '../../models/desktop-size';

    let username = '';
    let password = 'bayview1';
    let gatewayAddress = 'ws://localhost:7171/jet/fwd/tcp/91cf65b1-6d5b-4e4b-9560-67d4764aaadb';
    let hostname = '192.168.1.160:5900';
    let domain = '';
    let authtoken =
        `eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCIsImN0eSI6IkFTU09DSUFUSU9OIn0.eyJkc3RfaHN0IjoiMTkyLjE2OC4xLjE2MDo1OTAwIiwiZXhwIjoxNzA4MTE1Nzc3LCJqZXRfYWlkIjoiOTFjZjY1YjEtNmQ1Yi00ZTRiLTk1NjAtNjdkNDc2NGFhYWRiIiwiamV0X2FwIjoidm5jIiwiamV0X2NtIjoiZndkIiwianRpIjoiOTJkNGE1ZTctZjVlNC00ODBlLTlkODEtMjJmMjg2NGQ1YjQ4IiwibmJmIjoxNzA4MTE0ODc3fQ.EFDcqxhTm_QHC-vwe4H_vCryxxexL5EeRma25kuA-sVWLGJ9j-uNC9KpHiJ9Cg_L3KxW-MY9t5ecXJfkd-G_98wlvaVq6dK-jfKzmOKxqGJoJpUo7AVTiURD88tTHhwZHsZ8m8FybAgX_YtE8Zl9Su9Y4878JseVs4KurnXTiLJuHb12PmI6u3zuWaLQAlKWqytH7gcEuxCBIHb47VyAoJBxAWpwZU0hGGjSHgw58waz296TSv1azrOEAZ_UvJW7gy6MzDjegQxzlhyON5S_Cs8squX6pvU_IP7PBLCur8qoK3eEpvuMlDjXNGwrrjRor6QikdNfUzFAhkWjQvoJ7w`
    let kdc_proxy_url = '';
    let desktopSize: DesktopSize = {
        width: 1280,
        height: 768,
    };
    let pcb: string;

    let userInteraction: UserInteraction;

    userInteractionService.subscribe((val) => {
        userInteraction = val;
        if (val != null) {
            initListeners();
        }
    });

    const initListeners = () => {
        userInteraction.sessionListener.subscribe((event) => {
            if (event.type === 2) {
                console.log('Error event', event.data);

                toast.set({
                    type: 'error',
                    message: typeof event.data !== 'string' ? event.data.backtrace() : event.data,
                });
            } else {
                toast.set({
                    type: 'info',
                    message: typeof event.data !== 'string' ? event.data.backtrace() : event.data ?? 'No info',
                });
            }
        });
    };

    const StartSession = () => {
        toast.set({
            type: 'info',
            message: 'Connection in progress...',
        });
        userInteraction
            .connect(username, password, hostname, gatewayAddress, domain, authtoken, desktopSize, pcb, kdc_proxy_url)
            .pipe(
                catchError((err) => {
                    toast.set({
                        type: 'info',
                        message: err.backtrace(),
                    });
                    return of(null);
                }),
                filter((result) => !!result),
            )
            .subscribe((start_info: NewSessionInfo | null) => {
                if (start_info != null && start_info.initial_desktop_size !== null) {
                    toast.set({
                        type: 'info',
                        message: 'Success',
                    });
                    currentSession.update((session) =>
                        Object.assign(session, {
                            sessionId: start_info.session_id,
                            desktopSize: start_info.initial_desktop_size,
                            active: true,
                        }),
                    );
                    showLogin.set(false);
                } else {
                    toast.set({
                        type: 'error',
                        message: 'Failure',
                    });
                }
            });
    };
</script>

<main class="responsive">
    <div class="large-space" />
    <div class="grid">
        <div class="s2" />
        <div class="s8">
            <article class="primary-container">
                <h5>Login</h5>
                <div class="medium-space" />
                <div>
                    <div class="field label border">
                        <input id="hostname" type="text" bind:value={hostname} />
                        <label for="hostname">Hostname</label>
                    </div>
                    <div class="field label border">
                        <input id="domain" type="text" bind:value={domain} />
                        <label for="domain">Domain</label>
                    </div>
                    <div class="field label border">
                        <input id="username" type="text" bind:value={username} />
                        <label for="username">Username</label>
                    </div>
                    <div class="field label border">
                        <input id="password" type="password" bind:value={password} />
                        <label for="password">Password</label>
                    </div>
                    <div class="field label border">
                        <input id="gatewayAddress" type="text" bind:value={gatewayAddress} />
                        <label for="gatewayAddress">Gateway Address</label>
                    </div>
                    <div class="field label border">
                        <input id="authtoken" type="text" bind:value={authtoken} />
                        <label for="authtoken">AuthToken</label>
                    </div>
                    <div class="field label border">
                        <input id="pcb" type="text" bind:value={pcb} />
                        <label for="pcb">Pre Connection Blob</label>
                    </div>
                    <div class="field label border">
                        <input id="desktopSizeW" type="text" bind:value={desktopSize.width} />
                        <label for="desktopSizeW">Desktop Width</label>
                    </div>
                    <div class="field label border">
                        <input id="desktopSizeH" type="text" bind:value={desktopSize.height} />
                        <label for="desktopSizeH">Desktop Height</label>
                    </div>
                    <div class="field label border">
                        <input id="kdc_proxy_url" type="text" bind:value={kdc_proxy_url} />
                        <label for="kdc_proxy_url">KDC Proxy URL</label>
                    </div>
                </div>
                <nav class="center-align">
                    <button on:click={StartSession}>Login</button>
                </nav>
            </article>
        </div>
        <div class="s2" />
    </div>
</main>

<style>
    @import './login.css';
</style>
