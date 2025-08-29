import { get, writable } from 'svelte/store';

function createQueueStore<T>() {
    const store = writable<T[]>([]);

    return {
        subscribe: store.subscribe,

        enqueue(item: T) {
            store.update((queue) => [...queue, item]);
        },

        shift(): T | undefined {
            let first: T | undefined;
            store.update((queue) => {
                if (queue.length == 0) return queue;
                first = queue[0];
                return queue.slice(1);
            });
            return first;
        },

        length(): number {
            return get(store).length;
        },
    };
}

export const runWhenFocusedQueue = createQueueStore<() => void>();
