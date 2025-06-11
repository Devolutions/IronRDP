export type Callback<T> = (_: T) => void;

export class Observable<T> {
    constructor() {
        this.subscribers = [];
    }

    subscribers: Array<Callback<T>>;

    subscribe(cb: Callback<T>) {
        this.subscribers.push(cb);
    }

    publish(value: T) {
        for (const cb of this.subscribers) {
            cb(value);
        }
    }
}
