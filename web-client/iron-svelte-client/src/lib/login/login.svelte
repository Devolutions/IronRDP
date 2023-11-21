<script lang="ts">
    import { currentSession, userInteractionService } from '../../services/session.service';
    import { catchError, filter } from 'rxjs/operators';
    import type { UserInteraction, NewSessionInfo } from '../../../static/iron-remote-gui';
    import { of } from 'rxjs';
    import { toast } from '$lib/messages/message-store';
    import { showLogin } from '$lib/login/login-store';
    import type { DesktopSize } from '../../models/desktop-size';

    let username = "vmbox";
    let password = "vmbox";
    let gatewayAddress = "ws://localhost:7171/jet/rdp";
    let hostname = "192.168.1.54:3389";
    let domain = "";
    let authtoken = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCIsImN0eSI6IkFTU09DSUFUSU9OIn0.eyJkc3RfaHN0IjoiMTkyLjE2OC4xLjU0OjMzODkiLCJleHAiOjE3Mjg3MDAwNTUsImpldF9haWQiOiI4N2FlOTUyNS03NzFiLTRjNWItOTI5ZC0zMjU4YjY5MTc1NDMiLCJqZXRfYXAiOiJ1bmtub3duIiwiamV0X2NtIjoiZndkIiwianRpIjoiNDExNGU0NmEtZTM2ZS00MDNmLTliYWMtMDFiZmY4MDdmMzBiIiwibmJmIjoxNjk3MTQyNDU1fQ.dO_wppJF_zCz7aAGl9j0fBLiPuJQdcxxoSYD6ELRBY1OTzyXJNgjeJkoKqlX-SrNy2D-cfao8xyvg6a-Nr3Uct9a7HFhNO0C2msrLPuHaFakczG63siLa0qbh_H-PI2jBVMReGwkQghuFQBugYzLIL_liC98pZP5hBVlEbigbZs";
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
                    message: event.data.backtrace != null ? event.data.backtrace() : event.data,
                });
            } else {
                toast.set({
                    type: 'info',
                    message: event.data ?? 'No info',
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
