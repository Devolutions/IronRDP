<script lang="ts">
    import { currentSession, userInteractionService } from '../../services/session.service';
    import { catchError, filter } from 'rxjs/operators';
    import type { UserInteraction, NewSessionInfo } from '../../../static/iron-remote-gui';
    import { of } from 'rxjs';
    import { toast } from '$lib/messages/message-store';
    import { showLogin } from '$lib/login/login-store';
    import type { DesktopSize } from '../../models/desktop-size';

    let username = 'Administrator';
    let password = 'DevoLabs123!';
    let gatewayAddress = 'ws://localhost:7171/jet/rdp';
    let hostname = 'IT-HELP-DC.ad.it-help.ninja:3389';
    let domain = 'ad.it-help.ninja';
    let authtoken =
        'eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCIsImN0eSI6IkFTU09DSUFUSU9OIn0NCg.eyJpYXQiOjE2OTkyODg4MDAsIm5iZiI6MTY5OTI4ODgwMCwiZXhwIjoxNjk5ODkzNjAwLCJqdGkiOiI5NjczZmQwYi01MzhmLTQwNTUtYjRhMy1iOTVhZTYzYWMxMDAiLCJqZXRfYXAiOiJyZHAiLCJqZXRfY20iOiJmd2QiLCJqZXRfYWlkIjoiNzkxNTFjYjctZjUxNS00ZjJjLWI1ZDQtNTA1N2Q5ODc2NjY5IiwiZHN0X2hzdCI6InRjcDovL0lULUhFTFAtREMuYWQuaXQtaGVscC5uaW5qYTozMzg5In0NCg.XBAUzc3WNgcroMkDeQS93WIfnf1WgiTb6gjT-gEzkhE-98TZWC8Zmjeb5-IVBe9H3YY4srMzKczFLJVfTbRZA-UCQCC0GrQPsnnv8eJAVxP29Gz7jdALHeAKWyAfpH4FuGKFeMHJV0D97lHILn67x-q3pf77BKBI9CKMNOkaKIeZsh11YF1I0ZvDfGB9rWohlMTMvr4zBQ1FgGr_ZBekM8_EerRHBbKuqTZIHKGJVmObI9oV4m_2l32Rt7Fw1WwbpNHfFhbIQYnzl9_NdihDN9yF_I7qF2s45zuxykXst7YxHe1OJFg1R6hcNKHvu0kovlv9Vin2pYfOMuHmhd_5Vg';
    let kdc_proxy_url =
        'http://localhost:7171/jet/KdcProxy/eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCIsImN0eSI6IktEQyJ9.eyJleHAiOjE2OTkyODk3MDAsImp0aSI6ImM0MzA4Nzg4LWU2NzctNDJjYi04Mzk1LThhOTZmZjZkODc3ZiIsImtyYl9rZGMiOiJ0Y3A6Ly9JVC1IRUxQLURDLmFkLml0LWhlbHAubmluamE6ODgiLCJrcmJfcmVhbG0iOiJhZC5pdC1oZWxwLm5pbmphIiwibmJmIjoxNjk5Mjg4ODAwfQ.cb6pk5Fge1OXh3w9qLYGjBuiGveEyVJ4rh7TL0imqtMeuFudF2ShSrbuRq8fl_5gEada-qWCz7Oo6VhiJsycEw21DhW1tj5qjLTarz_QiZQXqgTHYwHdfI5A89Cm8d6sPkMZKMM61e0AL7S2DXWoIn9MzZwHLfNhd_Aa40fQB0YbEtgAzk0hu_ao6nsYCpK32tq7BY6tDoEc0TDatTR4ToYhqZs6scJCE1wlPVKswpaVfhTAWbnWmhVXkiZhTKyGLXr3EDovRNv2FSAGK0j0jszDbNPtEuH1VG_Wx8a4Kc033y2A-8AsPTZrHj1pRKldBnHap-Ii7DLddaRvO3p9jA';
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
