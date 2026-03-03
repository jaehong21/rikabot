import http from "node:http";

const HOST = "127.0.0.1";
const PORT = 8797;

function sendJson(res, statusCode, body) {
  const payload = JSON.stringify(body);
  res.writeHead(statusCode, {
    "Content-Type": "application/json",
    "Content-Length": Buffer.byteLength(payload),
  });
  res.end(payload);
}

function parseBody(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    req.on("data", (chunk) => chunks.push(chunk));
    req.on("end", () => {
      try {
        const raw = Buffer.concat(chunks).toString("utf8");
        if (!raw.trim()) {
          resolve({});
          return;
        }
        resolve(JSON.parse(raw));
      } catch (error) {
        reject(error);
      }
    });
    req.on("error", reject);
  });
}

function buildMockResponse(requestBody) {
  const messages = Array.isArray(requestBody?.messages) ? requestBody.messages : [];

  const lastUserMessage = [...messages]
    .reverse()
    .find(
      (message) =>
        message &&
        message.role === "user" &&
        typeof message.content === "string" &&
        message.content.trim().length > 0,
    );

  const promptText = lastUserMessage?.content?.trim() ?? "";
  const responseText = promptText ? `mock-e2e: ${promptText}` : "mock-e2e: empty";

  return {
    id: "chatcmpl-e2e",
    object: "chat.completion",
    created: Math.floor(Date.now() / 1000),
    model:
      typeof requestBody?.model === "string" && requestBody.model.trim()
        ? requestBody.model
        : "mock-model",
    choices: [
      {
        index: 0,
        message: {
          role: "assistant",
          content: responseText,
        },
        finish_reason: "stop",
      },
    ],
    usage: {
      prompt_tokens: 8,
      completion_tokens: 8,
      total_tokens: 16,
    },
  };
}

const server = http.createServer(async (req, res) => {
  if (req.method === "GET" && req.url === "/health") {
    sendJson(res, 200, { status: "ok" });
    return;
  }

  const path = req.url ?? "";
  if (req.method === "POST" && (path === "/chat/completions" || path === "/v1/chat/completions")) {
    try {
      const body = await parseBody(req);
      sendJson(res, 200, buildMockResponse(body));
    } catch (error) {
      sendJson(res, 400, {
        error: "Invalid JSON body",
        detail: error instanceof Error ? error.message : String(error),
      });
    }
    return;
  }

  sendJson(res, 404, { error: "Not Found" });
});

server.listen(PORT, HOST, () => {
  process.stdout.write(`Mock OpenAI server listening on http://${HOST}:${PORT}\n`);
});

function shutdown() {
  server.close(() => {
    process.exit(0);
  });
}

process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);
