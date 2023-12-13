<script lang="ts">
    import { currentSession, userInteractionService } from '../../services/session.service';
    import { catchError, filter } from 'rxjs/operators';
    import type { UserInteraction, NewSessionInfo } from '../../../static/iron-remote-gui';
    import { of } from 'rxjs';
    import { toast } from '$lib/messages/message-store';
    import { showLogin } from '$lib/login/login-store';
    import type { DesktopSize } from '../../models/desktop-size';

    let username = 'protecteduser';
    let password = 'Protected123!';
    let gatewayAddress = 'ws://localhost:7171/jet/rdp';
    let hostname = '10.10.0.3:3389';
    let domain = 'ad.it-help.ninja';
    let authtoken =
        `eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCIsImN0eSI6IkFTU09DSUFUSU9OIn0NCg.eyJpYXQiOjE3MDI0ODc5ODYsIm5iZiI6MTcwMjQ4Nzk4NiwiZXhwIjoxNzAzMDkyNzg2LCJqdGkiOiI0ZTI2MzJiYy1iOTg2LTQyMzQtOThlZi0yNmI1NDE5ZDc2MmEiLCJqZXRfYXAiOiJyZHAiLCJqZXRfY20iOiJmd2QiLCJqZXRfYWlkIjoiNzJiMjI0YTItODg0ZS00YmUyLWIzMDUtM2Q0YzEzM2ZjNTBjIiwiZHN0X2hzdCI6InRjcDovL0lULUhFTFAtREMuYWQuaXQtaGVscC5uaW5qYTozMzg5In0NCg.UGs63H1kof5odiKg2057MIDrfsklQtaDR1-pSp38IPUkmGy4SxBoI2cTuYq6WFXnxDRVVxcFkG97dyAR5iLw5vqMi8ZHhZdjyAjefdYoRWL30kL0jmKkytGg7a1-eIG2glvki1C04AiIHQoHa01FTv4pvVAsZl398DBXqouHENLeSZJBKYNNgAxeJPH_JXEbYccX4X6sNCfEIpXPb9Bb1RDyTD3hTn3oVeiWqJ3ws-KWCII6teCdYngA1VdLund22Pw3_6zzVGKKk2ASXTbu830UAsAO7mtXg8WmYL3SQN5Sq2xQDs94rFPQzScjOLiKzRPS8h4Xk_21GJcrDSahtg`
    let kdc_proxy_url = 'http://localhost:7171/jet/KdcProxy/eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCIsImN0eSI6IktEQyJ9.eyJleHAiOjE3MDI0ODg4ODYsImp0aSI6IjY5MTkyYTU0LTM4YzEtNDNiYS1hOTFmLTA1MmJhMTZlNzM2NyIsImtyYl9rZGMiOiJ0Y3A6Ly9JVC1IRUxQLURDLmFkLml0LWhlbHAubmluamE6ODgiLCJrcmJfcmVhbG0iOiJhZC5pdC1oZWxwLm5pbmphIiwibmJmIjoxNzAyNDg3OTg2fQ.L-PGB9O7r8m9MRDF4iFvMFbkJtlgoQQAHaVBf84BOwmPChAUuYK02za2Sh7KfVkXohxXyGKxZMZ1TNfn474D1ySrPf4lI6HzNg_5RgU-sLZtcP2txhMRgIQRs0hNjZU_Xoalg3_AePYSRn4UJI_ulhbJkJq7lWblN8QaVMN0lD6TvVNq47IQtcKEAV-vDmLaVhGY7yigFkcUKL90fiF9SzWn2fvc3XEk-4ix55yf_Y0im63-bhNLZ-JMi_4QdZhkum740I9OT0XY0Wm8NAfEoM0gtGslJ6faIbk4e0FXKkxDMYAwpcv3ovjtVlVle4rxxkaQxCuCDJL_R1qEE-53bQ';
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
