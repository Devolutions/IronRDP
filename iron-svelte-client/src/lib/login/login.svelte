<script>
	import { Login, beerui } from './login';
	import { serverBridge } from '../../services/services-injector';

	const StartSession = () => {
		Login.toastText = 'Connection in progress...';
	 	beerui('#toast');
		serverBridge.connect(Login.username, Login.password, Login.host).subscribe(start_info => {
      if (start_info.websocket_port && start_info.websocket_port > 0) {
        Login.toastText = 'Success';
	 	beerui('#toast');
        // this.currentSession.sessionId = start_info.session_id;
        // this.currentSession.desktopSize = start_info.initial_desktop_size;
        // this.currentSession.active = true;
      } else {
		Login.toastText = 'Failure';
	 	beerui('#toast');
      }
    })
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
						<input id="host" type="text" bind:value={Login.host} />
						<label for="host">Host</label>
					</div>
					<div class="field label border">
						<input id="username" type="text" bind:value={Login.username} />
						<label for="username">Username</label>
					</div>
					<div class="field label border">
						<input id="password" type="password" bind:value={Login.password} />
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
	<span>{Login.toastText}</span>
</div>

<style>
	@import './login.css';
</style>
