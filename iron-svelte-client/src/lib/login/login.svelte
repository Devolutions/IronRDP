<script>
	import { serverBridge } from '../../services/services-injector';
	import { currentSession } from '../../services/session.service';
	import { catchError, Observable } from 'rxjs';

	let username = "Administrator";
	let password = "DevoLabs123!";
	let host = "10.10.0.3:3389";
	
	let toastMessage;

	const StartSession = () => {
		toastMessage = 'Connection in progress...';
		ui('#toast');
		serverBridge.connect(username, password, host)
			.pipe(
				catchError(err => {
					console.log("error");
					return err;
				})
			)
			.subscribe((start_info) => {
			if (start_info.websocket_port && start_info.websocket_port > 0) {
				toastMessage = 'Success';
				ui('#toast');
				currentSession.update(session => Object.assign(session, {
					sessionId: start_info.session_id,
					desktopSize: start_info.initial_desktop_size,
					active: true
				}));
			} else {
				toastMessage = 'Failure';
				ui('#toast');
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
						<input id="host" type="text" bind:value={host} />
						<label for="host">Host</label>
					</div>
					<div class="field label border">
						<input id="username" type="text" bind:value={username} />
						<label for="username">Username</label>
					</div>
					<div class="field label border">
						<input id="password" type="password" bind:value={password} />
						<label for="password">Password</label>
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

<div id="toast" class="toast blue white-text">
	<i>info</i>
	<span>{toastMessage}</span>
</div>

<style>
	@import './login.css';
</style>
