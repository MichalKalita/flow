import crypto from "k6/crypto";
import encoding from "k6/encoding";

export function signHs256(payload, secret) {
  const header = encoding.b64encode(JSON.stringify({ alg: "HS256" }), "rawurl");
  const body = encoding.b64encode(JSON.stringify(payload), "rawurl");
  const data = `${header}.${body}`;
  const signature = crypto.hmac("sha256", secret, data, "base64rawurl");
  return `${data}.${signature}`;
}

export function applicationToken(sub, secret, ttlSeconds = 3600) {
  const now = Math.floor(Date.now() / 1000);
  return signHs256(
    {
      iss: "https://identity.example.com",
      aud: "application",
      sub,
      exp: now + ttlSeconds,
    },
    secret,
  );
}
