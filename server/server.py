import asyncio
import websockets

async def test_ws():
    uri = "ws://127.0.0.1:3030/ws"
    async with websockets.connect(uri) as websocket:
        # Send a message
        await websocket.send("Hello Server")
        print("Sent: Hello Server")

        # Receive a message
        if True :
            response = await websocket.recv()
            print("Received:", response)

asyncio.run(test_ws())
