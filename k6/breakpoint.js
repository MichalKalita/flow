import http from "k6/http";
import { check } from "k6";
import { Counter } from "k6/metrics";
import { applicationToken } from "./jwt.js";

const BASE_URL = (__ENV.BASE_URL || "http://127.0.0.1:80").replace(/\/$/, "");
const JWT_SECRET =
  __ENV.JWT_SECRET || "development-key-32-bytes-minimum-123456";

const overloaded = new Counter("http_overloaded");
const soldOut = new Counter("order_sold_out");

const USERS = [
  { sub: "idp:u1", userId: 1, deviceId: 1 },
  { sub: "idp:u2", userId: 2, deviceId: 2 },
  { sub: "idp:u3", userId: 3, deviceId: null },
];

const PUBLIC_CATALOGS = [
  "/demo/api/products",
  "/bookstore/api/books",
  "/library/api/works",
  "/gym/api/plans",
  "/gym/api/classes",
  "/jobs/api/listings",
  "/restaurant/api/menu",
  "/delivery/api/kitchens",
  "/hotel/api/rooms",
  "/school/api/courses",
  "/tracker/api/projects",
];

const PRODUCT_IDS = [1, 2, 3, 4];
const ORDERABLE_PRODUCTS = [1, 2, 3];
const PAYMENT_METHODS = ["CARD", "BANK"];
const COMMANDS = ["START", "STOP"];

const orderStatuses = http.expectedStatuses(201, 400);
const commandStatuses = http.expectedStatuses(202, 403);

export const options = {
  scenarios: {
    breakpoint: {
      executor: "ramping-arrival-rate",
      startRate: 50,
      timeUnit: "1s",
      preAllocatedVUs: 128,
      maxVUs: 2048,
      stages: [
        { duration: "15s", target: 200 },
        { duration: "20s", target: 1000 },
        { duration: "30s", target: 5000 },
        { duration: "45s", target: 15000 },
        { duration: "45s", target: 40000 },
      ],
    },
  },
};

let token;
let user;
let orderIds = [];

function pick(items) {
  return items[Math.floor(Math.random() * items.length)];
}

function authHeaders() {
  return {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  };
}

function record(response) {
  if (response.status === 503) {
    overloaded.add(1);
  }
  return response;
}

export function setup() {
  const response = http.get(`${BASE_URL}/demo/api/products`);
  if (response.status !== 200) {
    throw new Error(
      `API not reachable at ${BASE_URL}/demo/api/products (${response.status}): ${response.body}`,
    );
  }
  const products = response.json();
  if (!Array.isArray(products) || products.length === 0) {
    throw new Error("GET /demo/api/products returned no products");
  }
  return { productCount: products.length };
}

function ensureSession() {
  if (token) {
    return;
  }
  user = USERS[(__VU - 1) % USERS.length];
  token = applicationToken(user.sub, JWT_SECRET);
}

export default function () {
  ensureSession();
  const roll = Math.random();

  if (roll < 0.4) {
    const response = record(
      http.get(`${BASE_URL}/demo/api/products`, {
        tags: { name: "GET /demo/api/products" },
      }),
    );
    check(response, { catalog: (r) => r.status === 200 });
    return;
  }

  if (roll < 0.55) {
    const response = record(
      http.get(`${BASE_URL}/demo/api/products/${pick(PRODUCT_IDS)}/photos`, {
        tags: { name: "GET /demo/api/products/{productId}/photos" },
      }),
    );
    check(response, { photos: (r) => r.status === 200 });
    return;
  }

  if (roll < 0.65) {
    const response = record(
      http.get(`${BASE_URL}${pick(PUBLIC_CATALOGS)}`, {
        tags: { name: "GET public catalog" },
      }),
    );
    check(response, { hosted: (r) => r.status === 200 });
    return;
  }

  if (roll < 0.75) {
    const response = record(
      http.get(`${BASE_URL}/demo/api/users`, {
        headers: authHeaders(),
        tags: { name: "GET /demo/api/users" },
      }),
    );
    check(response, { users: (r) => r.status === 200 });
    return;
  }

  if (roll < 0.85) {
    const response = record(
      http.get(`${BASE_URL}/demo/api/users/${user.userId}/orders`, {
        headers: authHeaders(),
        tags: { name: "GET /demo/api/users/{userId}/orders" },
      }),
    );
    if (response.status === 200) {
      const list = response.json();
      if (Array.isArray(list) && list.length > 0) {
        orderIds = list.map((row) => row.id).filter((id) => id);
      }
    }
    check(response, { orders: (r) => r.status === 200 });
    return;
  }

  if (roll < 0.93 && orderIds.length > 0) {
    const response = record(
      http.get(`${BASE_URL}/demo/api/orders/${pick(orderIds)}`, {
        headers: authHeaders(),
        tags: { name: "GET /demo/api/orders/{orderId}" },
      }),
    );
    check(response, { order: (r) => r.status === 200 });
    return;
  }

  if (roll < 0.97 && user.deviceId) {
    if (Math.random() < 0.5) {
      const response = record(
        http.get(`${BASE_URL}/demo/api/devices/${user.deviceId}`, {
          headers: authHeaders(),
          tags: { name: "GET /demo/api/devices/{deviceId}" },
        }),
      );
      check(response, { device: (r) => r.status === 200 });
      return;
    }
    const response = record(
      http.post(
        `${BASE_URL}/demo/api/devices/${user.deviceId}/commands`,
        JSON.stringify({ action: pick(COMMANDS) }),
        {
          headers: authHeaders(),
          tags: { name: "POST /demo/api/devices/{deviceId}/commands" },
          responseCallback: commandStatuses,
        },
      ),
    );
    check(response, { command: (r) => r.status === 202 });
    return;
  }

  const response = record(
    http.post(
      `${BASE_URL}/demo/api/orders`,
      JSON.stringify({
        items: [{ productId: pick(ORDERABLE_PRODUCTS), quantity: 1 }],
        paymentMethod: pick(PAYMENT_METHODS),
      }),
      {
        headers: authHeaders(),
        tags: { name: "POST /demo/api/orders" },
        responseCallback: orderStatuses,
      },
    ),
  );
  if (response.status === 400) {
    soldOut.add(1);
  }
  check(response, {
    "order created or sold out": (r) => r.status === 201 || r.status === 400,
  });
}
