let ws;
const DEBUG = true;


(() => {
    ws = new WebSocket(`${document.documentURI.replace(/https?/, 'ws')}ws`);

    ws.onopen = e => {
        console.log("Websocket connection established!");
    }

    ws.onmessage = e => {
        if (!e.data.packet_type)
            console.error(`Invalid packet recieved: ${JSON.stringify(e.data)}`);

        if (DEBUG)
            console.log(`Packet recieved: ${JSON.stringify(e.data)}`);
    }
})();