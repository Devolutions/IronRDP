export type PlaybackState = {
    paused: boolean;   // user intent: true = wants to be paused
    waiting: boolean;  // buffer not ready: fetch in-flight or data gap
    seeking: boolean;  // playhead is mid-jump (always co-occurs with waiting)
    ended: boolean;    // recording reached end (elapsed >= duration or session_ended)
};
