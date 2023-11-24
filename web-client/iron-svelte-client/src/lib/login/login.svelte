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
    let hostname = '10.10.0.3:3389';
    let domain = '';
    let authtoken =
        'eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCIsImN0eSI6IkFTU09DSUFUSU9OIn0.eyJkc3RfaHN0IjoiMTkyLjE2OC41Ni4xMDE6MzM4OSIsImV4cCI6MTY5MzQyMzY1NSwiamV0X2FpZCI6IjMwNzZjZGIwLWYxNTctNDJlNy1iOWMzLThhMTdlNDFkYjYwNyIsImpldF9hcCI6InJkcCIsImpldF9jbSI6ImZ3ZCIsImp0aSI6IjAwYjY4OTY2LWJiYjAtNDU0NS05ZDZiLWRjNmFmMjAzNjY5MiIsIm5iZiI6MTY5MzQyMjc1NX0.SYQv4HtWQbdHMHgoCLYejCfO3TtsMAyjjILB6-Nir3mBznKiSad3POeLf02n05JFc5QhCeSGxspAaoNU7-znQFhHr0Tt0MnZJ1YMQt4UoR3PR2fTuUqv8M5TKdm4lKwCIjh73tTD001glTkXHaxuCQBTFCUSzfZhXDIqq5-CQueKtCrgJfYepJLmlvgH-ujGcxfXoGJGmeUy3Fmaijiy0uaC98j9GNCfnAd6JENmSAOkxfroMFhq601PSEizRbPzq2exDakfJ0EkaANz15udBX1a7NP-RyANHWQb8hp0rj6hyuyg1-vfUKYusw5qNUjAGXaWOjHC5bLgnqfE2V8Xnw';
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
