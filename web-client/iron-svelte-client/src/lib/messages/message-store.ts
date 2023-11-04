import type { Writable } from 'svelte/store';
import { writable } from 'svelte/store';

type ToastMessage = {
    message: string;
    type: 'info' | 'error';
};
export const toast: Writable<ToastMessage> = writable();
