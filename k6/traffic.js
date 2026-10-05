import http from "k6/http";
import { check, sleep } from "k6";
import { Counter, Rate } from "k6/metrics";
import { applicationToken } from "./jwt.js";

const BASE_URL = (__ENV.BASE_URL || "http://127.0.0.1:80").replace(/\/$/, "");
const JWT_SECRET =
  __ENV.JWT_SECRET || "development-key-32-bytes-minimum-123456";
const PROFILE = __ENV.PROFILE || "traffic";

const soldOut = new Counter("order_sold_out");
const overloaded = new Counter("http_overloaded");
const orderOk = new Rate("order_accepted");

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
const orderReadStatuses = http.expectedStatuses(200, 404);
const commandStatuses = http.expectedStatuses(202, 403);

const profiles = {
  smoke: {
    vus: 1,
    duration: "20s",
    thresholds: {
      http_req_failed: ["rate<0.01"],
      checks: ["rate>0.99"],
    },
  },
  traffic: {
    scenarios: {
      users: {
        executor: "ramping-vus",
        startVUs: 1,
        stages: [
          { duration: "20s", target: 8 },
          { duration: "1m", target: 8 },
          { duration: "15s", target: 0 },
        ],
        gracefulRampDown: "10s",
      },
    },
    thresholds: {
      http_req_failed: ["rate<0.05"],
      http_req_duration: ["p(95)<1500"],
      checks: ["rate>0.95"],
    },
  },
  stress: {
    scenarios: {
      users: {
        executor: "ramping-vus",
        startVUs: 2,
        stages: [
          { duration: "15s", target: 16 },
          { duration: "30s", target: 24 },
          { duration: "15s", target: 0 },
        ],
        gracefulRampDown: "10s",
      },
    },
    thresholds: {
      http_req_duration: ["p(95)<3000"],
    },
  },
};

if (!profiles[PROFILE]) {
  throw new Error(`Unknown PROFILE=${PROFILE} (use smoke, traffic, or stress)`);
}

export const options = profiles[PROFILE];

let lastOrderId = null;

function pick(items) {
  return items[Math.floor(Math.random() * items.length)];
}

function chance(p) {
  return Math.random() < p;
}

function authHeaders(token) {
  return {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  };
}

function get(path, params = {}) {
  const response = http.get(`${BASE_URL}${path}`, params);
  if (response.status === 503) {
    overloaded.add(1);
  }
  return response;
}

function post(path, body, params = {}) {
  const response = http.post(`${BASE_URL}${path}`, JSON.stringify(body), params);
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

export default function () {
  const user = pick(USERS);
  const token = applicationToken(user.sub, JWT_SECRET);

  const catalog = get("/demo/api/products", {
    tags: { name: "GET /demo/api/products" },
  });
  check(catalog, {
    "products catalog": (r) => r.status === 200 && Array.isArray(r.json()),
  });

  if (chance(0.5)) {
    const productId = pick(PRODUCT_IDS);
    const photos = get(`/demo/api/products/${productId}/photos`, {
      tags: { name: "GET /demo/api/products/{productId}/photos" },
    });
    check(photos, {
      "product photos": (r) => r.status === 200,
    });
  }

  if (chance(0.35)) {
    const path = pick(PUBLIC_CATALOGS);
    const other = get(path, { tags: { name: "GET public catalog" } });
    check(other, {
      "hosted catalog": (r) => r.status === 200,
    });
  }

  if (chance(0.6)) {
    const users = get("/demo/api/users", {
      headers: authHeaders(token),
      tags: { name: "GET /demo/api/users" },
    });
    check(users, {
      "signed-in users": (r) => r.status === 200,
    });

    const orders = get(`/demo/api/users/${user.userId}/orders`, {
      headers: authHeaders(token),
      tags: { name: "GET /demo/api/users/{userId}/orders" },
    });
    check(orders, {
      "user orders": (r) => r.status === 200 && Array.isArray(r.json()),
    });

    if (lastOrderId) {
      const order = get(`/demo/api/orders/${lastOrderId}`, {
        headers: authHeaders(token),
        tags: { name: "GET /demo/api/orders/{orderId}" },
        responseCallback: orderReadStatuses,
      });
      check(order, {
        "own order or missing": (r) => r.status === 200 || r.status === 404,
      });
    }
  }

  if (user.deviceId && chance(0.4)) {
    const device = get(`/demo/api/devices/${user.deviceId}`, {
      headers: authHeaders(token),
      tags: { name: "GET /demo/api/devices/{deviceId}" },
    });
    check(device, {
      "own device": (r) => r.status === 200,
    });

    const alerts = get(`/demo/api/device-alerts/${user.deviceId}`, {
      headers: authHeaders(token),
      tags: { name: "GET /demo/api/device-alerts/{deviceId}" },
    });
    check(alerts, {
      "device alerts": (r) => r.status === 200,
    });

    if (chance(0.5)) {
      const command = post(
        `/demo/api/devices/${user.deviceId}/commands`,
        { action: pick(COMMANDS) },
        {
          headers: authHeaders(token),
          tags: { name: "POST /demo/api/devices/{deviceId}/commands" },
          responseCallback: commandStatuses,
        },
      );
      check(command, {
        "device command": (r) => r.status === 202,
      });
    }
  }

  if (chance(0.15)) {
    const created = post(
      "/demo/api/orders",
      {
        items: [{ productId: pick(ORDERABLE_PRODUCTS), quantity: 1 }],
        paymentMethod: pick(PAYMENT_METHODS),
      },
      {
        headers: authHeaders(token),
        tags: { name: "POST /demo/api/orders" },
        responseCallback: orderStatuses,
      },
    );
    const accepted = created.status === 201;
    orderOk.add(accepted);
    if (created.status === 400) {
      soldOut.add(1);
    }
    check(created, {
      "order created or sold out": (r) => r.status === 201 || r.status === 400,
    });
    if (accepted) {
      const body = created.json();
      lastOrderId = body.order && body.order.id;
    }
  }

  sleep(0.3 + Math.random() * 0.9);
}
