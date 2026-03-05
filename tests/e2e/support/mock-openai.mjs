import http from "node:http";

const HOST = "127.0.0.1";
const PORT = 8797;

function sleep(ms) {
  return new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
}

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

function extractToolOutput(messages) {
  const lastToolMessage = [...messages].reverse().find((message) => {
    return (
      message && message.role === "tool" && typeof message.content === "string"
    );
  });

  if (!lastToolMessage) {
    return "";
  }

  try {
    const parsed = JSON.parse(lastToolMessage.content);
    if (typeof parsed?.content === "string") {
      return parsed.content;
    }
  } catch {
    // fall through
  }

  return String(lastToolMessage.content ?? "");
}

async function buildMockResponse(requestBody) {
  const messages = Array.isArray(requestBody?.messages)
    ? requestBody.messages
    : [];

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
  const model =
    typeof requestBody?.model === "string" && requestBody.model.trim()
      ? requestBody.model
      : "mock-model";

  if (promptText.startsWith("e2e-error-slow:")) {
    await sleep(1_200);
    return {
      statusCode: 500,
      body: {
        error: {
          message: `mock-e2e error for ${promptText}`,
          type: "mock_error",
        },
      },
    };
  }

  if (promptText.startsWith("e2e-slow:")) {
    await sleep(10_000);
    return {
      statusCode: 200,
      body: {
        id: "chatcmpl-e2e-slow",
        object: "chat.completion",
        created: Math.floor(Date.now() / 1000),
        model,
        choices: [
          {
            index: 0,
            message: {
              role: "assistant",
              content: `mock-e2e: ${promptText}`,
            },
            finish_reason: "stop",
          },
        ],
        usage: {
          prompt_tokens: 8,
          completion_tokens: 8,
          total_tokens: 16,
        },
      },
    };
  }

  if (promptText.startsWith("e2e-nav-slow:")) {
    await sleep(3_500);
    return {
      statusCode: 200,
      body: {
        id: "chatcmpl-e2e-nav-slow",
        object: "chat.completion",
        created: Math.floor(Date.now() / 1000),
        model,
        choices: [
          {
            index: 0,
            message: {
              role: "assistant",
              content: `mock-e2e: ${promptText}`,
            },
            finish_reason: "stop",
          },
        ],
        usage: {
          prompt_tokens: 8,
          completion_tokens: 8,
          total_tokens: 16,
        },
      },
    };
  }

  if (promptText.startsWith("e2e-queue-slow:")) {
    await sleep(1_200);
    return {
      statusCode: 200,
      body: {
        id: "chatcmpl-e2e-queue-slow",
        object: "chat.completion",
        created: Math.floor(Date.now() / 1000),
        model,
        choices: [
          {
            index: 0,
            message: {
              role: "assistant",
              content: `mock-e2e: ${promptText}`,
            },
            finish_reason: "stop",
          },
        ],
        usage: {
          prompt_tokens: 8,
          completion_tokens: 8,
          total_tokens: 16,
        },
      },
    };
  }

  if (promptText.startsWith("e2e-tool-approval:")) {
    const toolOutput = extractToolOutput(messages);

    if (!toolOutput) {
      return {
        statusCode: 200,
        body: {
          id: "chatcmpl-e2e-tool-1",
          object: "chat.completion",
          created: Math.floor(Date.now() / 1000),
          model,
          choices: [
            {
              index: 0,
              message: {
                role: "assistant",
                content: "Using a tool now.",
                tool_calls: [
                  {
                    id: "call-e2e-shell",
                    type: "function",
                    function: {
                      name: "shell",
                      arguments: JSON.stringify({
                        command: "echo e2e-tool-approved",
                      }),
                    },
                  },
                ],
              },
              finish_reason: "tool_calls",
            },
          ],
          usage: {
            prompt_tokens: 8,
            completion_tokens: 8,
            total_tokens: 16,
          },
        },
      };
    }

    return {
      statusCode: 200,
      body: {
        id: "chatcmpl-e2e-tool-2",
        object: "chat.completion",
        created: Math.floor(Date.now() / 1000),
        model,
        choices: [
          {
            index: 0,
            message: {
              role: "assistant",
              content: `mock-e2e: tool-output:${toolOutput.trim()}`,
            },
            finish_reason: "stop",
          },
        ],
        usage: {
          prompt_tokens: 8,
          completion_tokens: 8,
          total_tokens: 16,
        },
      },
    };
  }

  const responseText = promptText
    ? `mock-e2e: ${promptText}`
    : "mock-e2e: empty";

  return {
    statusCode: 200,
    body: {
      id: "chatcmpl-e2e",
      object: "chat.completion",
      created: Math.floor(Date.now() / 1000),
      model,
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
    },
  };
}

const server = http.createServer(async (req, res) => {
  if (req.method === "GET" && req.url === "/health") {
    sendJson(res, 200, { status: "ok" });
    return;
  }

  const path = req.url ?? "";
  if (
    req.method === "POST" &&
    (path === "/chat/completions" || path === "/v1/chat/completions")
  ) {
    try {
      const body = await parseBody(req);
      const response = await buildMockResponse(body);
      sendJson(res, response.statusCode, response.body);
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
  process.stdout.write(
    `Mock OpenAI server listening on http://${HOST}:${PORT}\n`,
  );
});

function shutdown() {
  server.close(() => {
    process.exit(0);
  });
}

process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);
