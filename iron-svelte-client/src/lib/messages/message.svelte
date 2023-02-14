<script lang="ts">
    import {toast} from '$lib/messages/message-store';

    let toastMessage: string;

    toast.subscribe(t => {
        if (t) {
            toastMessage = t.message;
            switch (t.type) {
                case 'info':
                    ui('#toast');
                    break;
                case 'error':
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
