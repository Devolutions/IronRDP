<script lang="ts">
    import { toast } from '$lib/messages/message-store';

    let toastMessage: string;

    toast.subscribe((t) => {
        if (t != null) {
            toastMessage = t.message;
            switch (t.type) {
                case 'info':
                    // FIXME: update beercss (the `ui` function comes from this library)
                    // eslint-disable-next-line @typescript-eslint/ban-ts-comment
                    // @ts-ignore
                    // eslint-disable-next-line no-undef
                    ui('#toast');
                    break;
                case 'error':
                    // FIXME: update beercss (the `ui` function comes from this library)
                    // eslint-disable-next-line @typescript-eslint/ban-ts-comment
                    // @ts-ignore
                    // eslint-disable-next-line no-undef
                    ui('#toast-error');
                    break;
            }
        }
    });
</script>

<div id="toast" class="toast blue white-text">
    <i>info</i>
    <span>{toastMessage}</span>
</div>

<div id="toast-error" class="toast red white-text">
    <i>Error</i>
    <span>{toastMessage}</span>
</div>
