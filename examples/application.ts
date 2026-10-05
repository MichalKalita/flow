// Přepis examples/application.flow; návrhové API Flow není implementované.
// bind/ref označují symboly scénáře; vnořené kroky se deklarují, nevykonávají při registraci.
// Zachovává pravidla současného Flow včetně jeho neomezených dotazů a obecných číselných typů.

Flow.type("UserID", Flow.string(), Flow.greater(Flow.length(Flow.ref("value")), 0));

Flow.type("Quantity", Flow.integer(), Flow.between(Flow.ref("value"), 1, 100));

Flow.type("Stock", Flow.integer(), Flow.greaterEqual(Flow.ref("value"), 0));

Flow.type("Price", Flow.integer(), Flow.greaterEqual(Flow.ref("value"), 0));

Flow.type("CartItem", {"product_id": Flow.string(), "quantity": Flow.named("Quantity")});

Flow.type("Cart", Flow.listOf(Flow.named("CartItem")), Flow.between(Flow.length(Flow.ref("value")), 1, 50));

Flow.type("PaymentMethod", Flow.string(), Flow.member(Flow.ref("value"), ["card", "bank"]));

Flow.type(
    "Product",
    {
        "id": Flow.string(),
        "name": Flow.string(),
        "price_cents": Flow.named("Price"),
        "stock": Flow.named("Stock")
    }
);

Flow.type(
    "User",
    {"id": Flow.string(), "name": Flow.string(), "email": Flow.string(), "country": Flow.string()}
);

Flow.type(
    "OrderItem",
    {"id": Flow.string(), "name": Flow.string(), "price_cents": Flow.named("Price"), "quantity": Flow.integer()}
);

Flow.type(
    "Order",
    {
        "id": Flow.string(),
        "user_id": Flow.string(),
        "request_id": Flow.string(),
        "created_at": Flow.string(),
        "items": Flow.listOf(Flow.named("OrderItem")),
        "total_cents": Flow.integer(),
        "payment_method": Flow.named("PaymentMethod"),
        "payment_url": Flow.string(),
        "status": Flow.string()
    }
);

Flow.table("Products", Flow.named("Product"));

Flow.table("Users", Flow.named("User"));

Flow.table("Orders", Flow.named("Order"));

Flow.seed(
    "Products",
    [
        {"id": "p1", "name": "Studio sluchátka", "price_cents": 249000, "stock": 12},
        {"id": "p2", "name": "Mechanická klávesnice", "price_cents": 329000, "stock": 5},
        {"id": "p3", "name": "USB-C rozbočovač", "price_cents": 129000, "stock": 2},
        {"id": "p4", "name": "Webkamera", "price_cents": 189000, "stock": 0}
    ]
);

Flow.seed(
    "Users",
    [
        {"id": "u1", "name": "Petra Nováková", "email": "petra@example.test", "country": "CZ"},
        {"id": "u2", "name": "David Miller", "email": "david@example.test", "country": "US"},
        {"id": "u3", "name": "Nora Silva", "email": "nora@example.test", "country": "BR"}
    ]
);

Flow.type("DeviceID", Flow.string(), Flow.between(Flow.length(Flow.ref("value")), 1, 64));

Flow.type("Battery", Flow.integer(), Flow.between(Flow.ref("value"), 0, 100));

Flow.type("Latitude", Flow.number(), Flow.between(Flow.ref("value"), -90, 90));

Flow.type("Longitude", Flow.number(), Flow.between(Flow.ref("value"), -180, 180));

Flow.mqtt(
    "DeviceStatus",
    Flow.topic("devices/{device_id}/status"),
    Flow.input("device_id", Flow.named("DeviceID")),
    Flow.payload({"online": Flow.boolean(), "battery": Flow.named("Battery")}),
    Flow.history(24, "hours")
);

Flow.mqtt(
    "DevicePosition",
    Flow.topic("devices/{device_id}/position"),
    Flow.input("device_id", Flow.named("DeviceID")),
    Flow.payload({"latitude": Flow.named("Latitude"), "longitude": Flow.named("Longitude")}),
    Flow.history(5, "minutes")
);

Flow.table(
    "DeviceAlerts",
    {
        "id": Flow.string(),
        "device_id": Flow.named("DeviceID"),
        "kind": Flow.string(),
        "created_at": Flow.string()
    }
);

Flow.onMqtt(
    "DeviceStatus",
    Flow.when(
        Flow.less(Flow.ref("message.battery"), 20),
        Flow.transaction(
            Flow.insert(
                "DeviceAlerts",
                {
                    "id": Flow.uuid("alert"),
                    "device_id": Flow.ref("input.device_id"),
                    "kind": "low_battery",
                    "created_at": Flow.ref("request.time")
                }
            ),
            Flow.commit()
        )
    ),
    Flow.result({"accepted": true})
);

Flow.http(
    "GET",
    "/api/device-alerts/:device_id",
    Flow.input("device_id", Flow.named("DeviceID")),
    Flow.bind(
        "alerts",
        Flow.query("DeviceAlerts", "a", Flow.equal(Flow.ref("a.device_id"), Flow.ref("input.device_id")))
    ),
    Flow.result({"alerts": Flow.ref("alerts")})
);

Flow.http(
    "GET",
    "/api/devices/:device_id",
    Flow.input("device_id", Flow.named("DeviceID")),
    Flow.bind("status", Flow.last(Flow.source("DeviceStatus", Flow.ref("input.device_id")))),
    Flow.bind(
        "positions",
        Flow.orderBy(
            Flow.querySource(
                "DevicePosition",
                Flow.ref("input.device_id"),
                "p",
                Flow.greaterEqual(Flow.ref("p.received_at"), Flow.ago(5, "minutes"))
            ),
            "p.received_at",
            "asc"
        )
    ),
    Flow.result(
        {
            "device_id": Flow.ref("input.device_id"),
            "status": Flow.ref("status"),
            "positions": Flow.ref("positions")
        }
    )
);

Flow.http(
    "POST",
    "/api/orders",
    Flow.input("user_id", Flow.named("UserID")),
    Flow.input("items", Flow.named("Cart")),
    Flow.input("payment_method", Flow.named("PaymentMethod")),
    Flow.input("email_failure", Flow.named("Bool"), false),
    Flow.transaction(
        Flow.bind(
            "user",
            Flow.one(Flow.query("Users", "u", Flow.equal(Flow.ref("u.id"), Flow.ref("input.user_id"))))
        ),
        Flow.ensure(Flow.present(Flow.ref("user")), Flow.error(404, "user_not_found", "Uživatel neexistuje.")),
        Flow.bind("cart", Flow.groupSum(Flow.ref("input.items"), "product_id", "quantity")),
        Flow.bind(
            "snapshots",
            Flow.typed(
                Flow.listOf(Flow.named("OrderItem")),
                Flow.mapEach(
                    "item",
                    Flow.ref("cart"),
                    Flow.bind(
                        "product",
                        Flow.one(
                            Flow.query(
                                "Products",
                                "p",
                                Flow.equal(Flow.ref("p.id"), Flow.ref("item.product_id"))
                            )
                        )
                    ),
                    Flow.ensure(
                        Flow.present(Flow.ref("product")),
                        Flow.error(404, "product_not_found", "Produkt neexistuje.")
                    ),
                    Flow.result(
                        {
                            "id": Flow.ref("product.id"),
                            "name": Flow.ref("product.name"),
                            "price_cents": Flow.ref("product.price_cents"),
                            "quantity": Flow.ref("item.quantity")
                        }
                    )
                )
            )
        ),
        Flow.bind(
            "total",
            Flow.typed(
                Flow.integer(),
                Flow.sum(
                    "item",
                    Flow.ref("snapshots"),
                    Flow.multiply(Flow.ref("item.price_cents"), Flow.ref("item.quantity"))
                )
            )
        ),
        Flow.bind(
            "order",
            Flow.typed(
                Flow.named("Order"),
                Flow.insert(
                    "Orders",
                    {
                        "id": Flow.uuid("ord"),
                        "user_id": Flow.ref("user.id"),
                        "request_id": Flow.ref("request.id"),
                        "created_at": Flow.ref("request.time"),
                        "items": Flow.ref("snapshots"),
                        "total_cents": Flow.ref("total"),
                        "payment_method": Flow.ref("input.payment_method"),
                        "payment_url": "",
                        "status": "awaiting_payment"
                    }
                )
            )
        ),
        Flow.each(
            "item",
            Flow.ref("snapshots"),
            Flow.bind(
                "product",
                Flow.one(Flow.query("Products", "p", Flow.equal(Flow.ref("p.id"), Flow.ref("item.id"))))
            ),
            Flow.ensure(
                Flow.present(Flow.ref("product")),
                Flow.error(404, "product_not_found", "Produkt neexistuje.")
            ),
            Flow.ensure(
                Flow.greaterEqual(Flow.ref("product.stock"), Flow.ref("item.quantity")),
                Flow.error(409, "insufficient_stock", "Nedostatek kusů na skladě.")
            ),
            Flow.update(
                Flow.query("Products", "p", Flow.equal(Flow.ref("p.id"), Flow.ref("item.id"))),
                {"stock": Flow.subtract(Flow.ref("p.stock"), Flow.ref("item.quantity"))}
            )
        ),
        Flow.bind(
            "payment",
            Flow.call(
                "Payment.create_url",
                {
                    "order_id": Flow.ref("order.id"),
                    "amount_cents": Flow.ref("order.total_cents"),
                    "currency": "CZK",
                    "country": Flow.ref("user.country"),
                    "method": Flow.ref("input.payment_method")
                }
            )
        ),
        Flow.update(
            Flow.query("Orders", "o", Flow.equal(Flow.ref("o.id"), Flow.ref("order.id"))),
            {"payment_url": Flow.ref("payment.url")}
        ),
        Flow.queue(
            "Email.send_confirmation",
            {
                "order_id": Flow.ref("order.id"),
                "to": Flow.ref("user.email"),
                "subject": Flow.concat(["Objednávka ", Flow.ref("order.id")]),
                "payment_url": Flow.ref("payment.url"),
                "total_cents": Flow.ref("order.total_cents"),
                "items": Flow.ref("snapshots"),
                "simulate_failure": Flow.ref("input.email_failure")
            },
            Flow.policy(Flow.attempts(3), Flow.delay(1000, "ms"), Flow.retain())
        ),
        Flow.commit()
    ),
    Flow.response(
        201,
        {
            "order": Flow.merge(Flow.ref("order"), {"currency": "CZK", "payment_url": Flow.ref("payment.url")}),
            "payment": Flow.ref("payment"),
            "email": {"state": "queued"}
        }
    )
);

Flow.http(
    "GET",
    "/api/orders/:order_id",
    Flow.input("order_id", Flow.named("String")),
    Flow.bind(
        "order",
        Flow.one(Flow.query("Orders", "o", Flow.equal(Flow.ref("o.id"), Flow.ref("input.order_id"))))
    ),
    Flow.ensure(Flow.present(Flow.ref("order")), Flow.error(404, "order_not_found", "Objednávka neexistuje.")),
    Flow.result(Flow.ref("order"))
);

Flow.type(
    "ProductPhoto",
    Flow.image(),
    Flow.all(
        Flow.lessEqual(Flow.ref("value.byte_size"), 10485760),
        Flow.lessEqual(Flow.ref("value.width"), 8000),
        Flow.lessEqual(Flow.ref("value.height"), 8000)
    )
);

Flow.type("PhotoSize", Flow.integer(), Flow.between(Flow.ref("value"), 1, 4096));

Flow.table(
    "Photos",
    {
        "id": Flow.string(),
        "product_id": Flow.string(),
        "file_id": Flow.string(),
        "url": Flow.string(),
        "width": Flow.integer(),
        "height": Flow.integer()
    }
);

Flow.http(
    "POST",
    "/api/products/:product_id/photo",
    Flow.input("product_id", Flow.named("String")),
    Flow.input("photo", Flow.named("ProductPhoto")),
    Flow.input("width", Flow.named("PhotoSize"), 800),
    Flow.input("height", Flow.named("PhotoSize"), 600),
    Flow.transaction(
        Flow.bind(
            "product",
            Flow.one(Flow.query("Products", "p", Flow.equal(Flow.ref("p.id"), Flow.ref("input.product_id"))))
        ),
        Flow.ensure(
            Flow.present(Flow.ref("product")),
            Flow.error(404, "product_not_found", "Produkt neexistuje.")
        ),
        Flow.bind(
            "resized",
            Flow.typed(
                Flow.named("ProductPhoto"),
                Flow.call(
                    "Image.resize",
                    {
                        "image": Flow.ref("input.photo"),
                        "width": Flow.ref("input.width"),
                        "height": Flow.ref("input.height")
                    }
                )
            )
        ),
        Flow.bind("stored", Flow.call("Files.put", {"file": Flow.ref("resized")})),
        Flow.bind(
            "saved",
            Flow.insert(
                "Photos",
                {
                    "id": Flow.uuid("photo"),
                    "product_id": Flow.ref("input.product_id"),
                    "file_id": Flow.ref("stored.id"),
                    "url": Flow.ref("stored.url"),
                    "width": Flow.ref("resized.width"),
                    "height": Flow.ref("resized.height")
                }
            )
        ),
        Flow.commit()
    ),
    Flow.response(201, {"photo": Flow.ref("saved"), "file": Flow.ref("stored")})
);

Flow.http(
    "GET",
    "/api/products/:product_id/photos",
    Flow.input("product_id", Flow.named("String")),
    Flow.bind(
        "photos",
        Flow.query("Photos", "p", Flow.equal(Flow.ref("p.product_id"), Flow.ref("input.product_id")))
    ),
    Flow.result({"photos": Flow.ref("photos")})
);

Flow.table(
    "DeviceAccess",
    {
        "id": Flow.string(),
        "user_id": Flow.named("UserID"),
        "token": Flow.string(),
        "device_id": Flow.named("DeviceID")
    }
);

Flow.seed(
    "DeviceAccess",
    [
        {"id": "petra-mower1", "user_id": "u1", "token": "demo-petra", "device_id": "mower1"},
        {"id": "david-mower2", "user_id": "u2", "token": "demo-david", "device_id": "mower2"}
    ]
);

Flow.websocket(
    "/ws",
    Flow.input("token", Flow.named("String")),
    Flow.authorize(
        Flow.exists(
            Flow.query(
                "DeviceAccess",
                "access",
                Flow.all(
                    Flow.equal(Flow.ref("access.token"), Flow.ref("input.token")),
                    Flow.exists(
                        Flow.query("Users", "user", Flow.equal(Flow.ref("user.id"), Flow.ref("access.user_id")))
                    )
                )
            )
        )
    ),
    Flow.allowSource(
        "DeviceStatus",
        Flow.exists(
            Flow.query(
                "DeviceAccess",
                "access",
                Flow.all(
                    Flow.equal(Flow.ref("access.token"), Flow.ref("input.token")),
                    Flow.equal(Flow.ref("access.device_id"), Flow.ref("input.device_id"))
                )
            )
        )
    ),
    Flow.allowSource(
        "DevicePosition",
        Flow.exists(
            Flow.query(
                "DeviceAccess",
                "access",
                Flow.all(
                    Flow.equal(Flow.ref("access.token"), Flow.ref("input.token")),
                    Flow.equal(Flow.ref("access.device_id"), Flow.ref("input.device_id"))
                )
            )
        )
    )
);

Flow.type("MowerAction", Flow.string(), Flow.member(Flow.ref("value"), ["start", "stop"]));

Flow.mqtt(
    "DeviceCommand",
    Flow.topic("devices/{device_id}/command"),
    Flow.input("device_id", Flow.named("DeviceID")),
    Flow.payload({"command_id": Flow.string(), "action": Flow.named("MowerAction")}),
    Flow.history(24, "hours")
);

Flow.http(
    "POST",
    "/api/devices/:device_id/commands",
    Flow.input("device_id", Flow.named("DeviceID")),
    Flow.input("action", Flow.named("MowerAction")),
    Flow.input("token", Flow.named("String")),
    Flow.ensure(
        Flow.exists(
            Flow.query(
                "DeviceAccess",
                "access",
                Flow.all(
                    Flow.equal(Flow.ref("access.token"), Flow.ref("input.token")),
                    Flow.equal(Flow.ref("access.device_id"), Flow.ref("input.device_id")),
                    Flow.exists(
                        Flow.query("Users", "user", Flow.equal(Flow.ref("user.id"), Flow.ref("access.user_id")))
                    )
                )
            )
        ),
        Flow.error(403, "forbidden", "Přístup k zařízení je zamítnut.")
    ),
    Flow.transaction(
        Flow.bind(
            "outgoing",
            Flow.publish(
                Flow.source("DeviceCommand", Flow.ref("input.device_id")),
                {"command_id": Flow.uuid("command"), "action": Flow.ref("input.action")}
            )
        ),
        Flow.commit()
    ),
    Flow.response(202, Flow.ref("outgoing"))
);
