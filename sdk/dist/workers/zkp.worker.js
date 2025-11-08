import { computeNovaProof, computeSingleTeleportProof } from '../zkp/runtime.js';
const ctx = self;
ctx.addEventListener('message', async (event) => {
    const message = event.data;
    if (!message) {
        return;
    }
    try {
        let payload;
        if (message.type === 'singleTeleport') {
            payload = await computeSingleTeleportProof(message.payload);
        }
        else if (message.type === 'nova') {
            payload = await computeNovaProof(message.payload);
        }
        else {
            throw new Error(`Unknown ZKP worker job type ${message.type}`);
        }
        const response = { id: message.id, type: 'result', result: payload };
        ctx.postMessage(response);
    }
    catch (error) {
        const response = {
            id: message.id,
            type: 'error',
            error: {
                message: error instanceof Error ? error.message : String(error),
                stack: error instanceof Error ? error.stack : undefined,
            },
        };
        ctx.postMessage(response);
    }
});
//# sourceMappingURL=zkp.worker.js.map