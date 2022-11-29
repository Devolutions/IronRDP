// Using separate ts file can be tricky because bind is not updated when value is mutated in function
// See this link to understand how it works under the hood: https://lihautan.com/compile-svelte-in-your-head-part-2
// We are supposed to be able to make a real separation with svelte-preprocess: https://github.com/sveltejs/svelte-preprocess#external-files

declare const ui: any;
class LoginPage {
	username = '';
	password = '';
	host = '';
	toastText = '';
}

export const beerui = ui;

export const Login = new LoginPage();