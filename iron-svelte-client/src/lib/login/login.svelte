<script lang="ts">
    import {currentSession, userInteractionService} from '../../services/session.service';
    import {catchError, filter} from "rxjs/operators";
    import type {IRGUserInteraction, NewSessionInfo} from '../../../static/iron-remote-gui';
    import {of} from 'rxjs';
    import {toast} from '$lib/messages/message-store';
    import {showLogin} from '$lib/login/login-store';

    let username = "Administrator";
    let password = "DevoLabs123!";
    let host = "ws://localhost:7172/jet/rdp"; //"10.10.0.3:3389";
    let authtoken = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCIsImN0eSI6IkFTU09DSUFUSU9OIn0.eyJkc3RfaHN0IjoiMTAuMTAuMC4zIiwiZXhwIjoxNjcxMTIwNjE0LCJqZXRfYWlkIjoiOWMwODAwNTktMDMzOS00MjJhLTgxODgtODEzNGJjOTc3MzczIiwiamV0X2FwIjoicmRwIiwiamV0X2NtIjoiZndkIiwianRpIjoiOTAzMDNlMDUtMzg5MC00OGQ3LTgxMTYtYWJmYzAwYWNlMTUxIiwibmJmIjoxNjcxMTE5NzE0fQ.JdT4KSyB2Zf3OcEA44Hmmc59cqx6KApXrFoJf_gIQwU8VqYWMnSMqENMyYw4CLDKj31tgUlSjWkHLj2wELZCOWFtsbONJqTWIc8mkCpnlbGVWIaNm7MISZXAS2p1LF1nsv9kzCJNvWK2AgfjsiZ4TBIUrhLa1dCRfuLsNaABotjcTJFvVCZUaadejeFDA6S2YbvQQHOjztIKJsg3zKkvTOpB_cZvRv9yDSgW09wXS0MOsnLqzmiLMd-9IPEkkwQ4oe9e6-AJI3OXZogkJDTcE0xdHlMSUG6JVwowt9FHervTn1n3nuN1ZKARvDbEsHJsLxPI1w2eqlZvPkqfKw5oqA";

    let toastMessage: string;

    let userInteraction: IRGUserInteraction;

    userInteractionService.subscribe(val => {
        userInteraction = val;
        if (val) {
            initListeners();
        }
    });

    const initListeners = () => {
        userInteraction.sessionListener.subscribe(event => {
            if (event.type === 2) {
                console.log("Error event", event.data);

                toast.set({
                    type: 'error',
                    message: event.data.backtrace ? event.data.backtrace() : event.data,
                });
            } else {
                toast.set({
                    type: 'info',
                    message: event.data || "No info",
                });
            }
        });
    }

    const StartSession = () => {
        toast.set({
            type: 'info',
            message: 'Connection in progress...'
        });
        userInteraction.connect(username, password, host, authtoken)
            .pipe(
                catchError(err => {
                    toast.set({
                        type: 'info',
                        message: err.backtrace()
                    });
                    return of(null);
                }),
                filter(result => !!result)
            )
            .subscribe((start_info: NewSessionInfo) => {

                if (import.meta.env.MODE === 'tauri' && start_info.websocket_port && start_info.websocket_port > 0) { //Tauri only
                    toast.set({
                        type: 'info',
                        message: 'Success'
                    });
                    currentSession.update(session => Object.assign(session, {
                        sessionId: start_info.session_id,
                        desktopSize: start_info.initial_desktop_size,
                        active: true
                    }));
                    showLogin.set(false);
                } else if (start_info.initial_desktop_size !== null) { //Browser
                    toast.set({
                        type: 'info',
                        message: 'Success',
                    });
                    currentSession.update(session => Object.assign(session, {
                        sessionId: start_info.session_id,
                        desktopSize: start_info.initial_desktop_size,
                        active: true,
                    }));
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
    <div class="large-space"/>
    <div class="grid">
        <div class="s2"/>
        <div class="s8">
            <article class="primary-container">
                <h5>Login</h5>
                <div class="medium-space"/>
                <div>
                    <div class="field label border">
                        <input id="host" type="text" bind:value={host}/>
                        <label for="host">Host</label>
                    </div>
                    <div class="field label border">
                        <input id="username" type="text" bind:value={username}/>
                        <label for="username">Username</label>
                    </div>
                    <div class="field label border">
                        <input id="password" type="password" bind:value={password}/>
                        <label for="password">Password</label>
                    </div>
                    <div class="field label border">
                        <input id="authtoken" type="text" bind:value={authtoken}/>
                        <label for="authtoken">AuthToken</label>
                    </div>
                </div>
                <nav class="center-align">
                    <button on:click={StartSession}>Login</button>
                </nav>
            </article>
        </div>
        <div class="s2"/>
    </div>
</main>


<style>
    @import './login.css';
</style>
