<script>
	import './login.ts';
</script>

<main class="responsive">
    <div class="large-space"></div>
	<div class="grid">
		<div class="s2" />
		<div class="s8">
			<article class="primary-container">
				<h5>Login</h5>
                <div class="medium-space"></div>
				<div>
					<div class="field label border">
						<input id="host" type="text" />
						<label for="host">Host</label>
					</div>
					<div class="field label border">
						<input id="username" type="text" />
						<label for="username">Username</label>
					</div>
					<div class="field label border">
						<input id="password" type="password" />
						<label for="password">Password</label>
					</div>
				</div>
				<nav class="center-align">
					<button>Login</button>
				</nav>
			</article>
		</div>
		<div class="s2" />
	</div>
</main>

<style>
	@import './login.css';
</style>
