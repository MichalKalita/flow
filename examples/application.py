# Přepis examples/application.flow; návrhové API Flow není implementované.
# bind/ref označují symboly scénáře; vnořené kroky se deklarují, nevykonávají při registraci.
# Zachovává pravidla současného Flow včetně jeho neomezených dotazů a obecných číselných typů.

flow.type("UserID", flow.string(), flow.greater(flow.length(flow.ref("value")), 0))

flow.type("Quantity", flow.integer(), flow.between(flow.ref("value"), 1, 100))

flow.type("Stock", flow.integer(), flow.greater_equal(flow.ref("value"), 0))

flow.type("Price", flow.integer(), flow.greater_equal(flow.ref("value"), 0))

flow.type("CartItem", {"product_id": flow.string(), "quantity": flow.named("Quantity")})

flow.type("Cart", flow.list_of(flow.named("CartItem")), flow.between(flow.length(flow.ref("value")), 1, 50))

flow.type("PaymentMethod", flow.string(), flow.member(flow.ref("value"), ["card", "bank"]))

flow.type(
    "Product",
    {
        "id": flow.string(),
        "name": flow.string(),
        "price_cents": flow.named("Price"),
        "stock": flow.named("Stock")
    }
)

flow.type(
    "User",
    {"id": flow.string(), "name": flow.string(), "email": flow.string(), "country": flow.string()}
)

flow.type(
    "OrderItem",
    {"id": flow.string(), "name": flow.string(), "price_cents": flow.named("Price"), "quantity": flow.integer()}
)

flow.type(
    "Order",
    {
        "id": flow.string(),
        "user_id": flow.string(),
        "request_id": flow.string(),
        "created_at": flow.string(),
        "items": flow.list_of(flow.named("OrderItem")),
        "total_cents": flow.integer(),
        "payment_method": flow.named("PaymentMethod"),
        "payment_url": flow.string(),
        "status": flow.string()
    }
)

flow.table("Products", flow.named("Product"))

flow.table("Users", flow.named("User"))

flow.table("Orders", flow.named("Order"))

flow.seed(
    "Products",
    [
        {"id": "p1", "name": "Studio sluchátka", "price_cents": 249000, "stock": 12},
        {"id": "p2", "name": "Mechanická klávesnice", "price_cents": 329000, "stock": 5},
        {"id": "p3", "name": "USB-C rozbočovač", "price_cents": 129000, "stock": 2},
        {"id": "p4", "name": "Webkamera", "price_cents": 189000, "stock": 0}
    ]
)

flow.seed(
    "Users",
    [
        {"id": "u1", "name": "Petra Nováková", "email": "petra@example.test", "country": "CZ"},
        {"id": "u2", "name": "David Miller", "email": "david@example.test", "country": "US"},
        {"id": "u3", "name": "Nora Silva", "email": "nora@example.test", "country": "BR"}
    ]
)

flow.type("DeviceID", flow.string(), flow.between(flow.length(flow.ref("value")), 1, 64))

flow.type("Battery", flow.integer(), flow.between(flow.ref("value"), 0, 100))

flow.type("Latitude", flow.number(), flow.between(flow.ref("value"), -90, 90))

flow.type("Longitude", flow.number(), flow.between(flow.ref("value"), -180, 180))

flow.mqtt(
    "DeviceStatus",
    flow.topic("devices/{device_id}/status"),
    flow.input("device_id", flow.named("DeviceID")),
    flow.payload({"online": flow.boolean(), "battery": flow.named("Battery")}),
    flow.history(24, "hours")
)

flow.mqtt(
    "DevicePosition",
    flow.topic("devices/{device_id}/position"),
    flow.input("device_id", flow.named("DeviceID")),
    flow.payload({"latitude": flow.named("Latitude"), "longitude": flow.named("Longitude")}),
    flow.history(5, "minutes")
)

flow.table(
    "DeviceAlerts",
    {
        "id": flow.string(),
        "device_id": flow.named("DeviceID"),
        "kind": flow.string(),
        "created_at": flow.string()
    }
)

flow.on_mqtt(
    "DeviceStatus",
    flow.when(
        flow.less(flow.ref("message.battery"), 20),
        flow.transaction(
            flow.insert(
                "DeviceAlerts",
                {
                    "id": flow.uuid("alert"),
                    "device_id": flow.ref("input.device_id"),
                    "kind": "low_battery",
                    "created_at": flow.ref("request.time")
                }
            ),
            flow.commit()
        )
    ),
    flow.result({"accepted": True})
)

flow.http(
    "GET",
    "/api/device-alerts/:device_id",
    flow.input("device_id", flow.named("DeviceID")),
    flow.bind(
        "alerts",
        flow.query("DeviceAlerts", "a", flow.equal(flow.ref("a.device_id"), flow.ref("input.device_id")))
    ),
    flow.result({"alerts": flow.ref("alerts")})
)

flow.http(
    "GET",
    "/api/devices/:device_id",
    flow.input("device_id", flow.named("DeviceID")),
    flow.bind("status", flow.last(flow.source("DeviceStatus", flow.ref("input.device_id")))),
    flow.bind(
        "positions",
        flow.order_by(
            flow.query_source(
                "DevicePosition",
                flow.ref("input.device_id"),
                "p",
                flow.greater_equal(flow.ref("p.received_at"), flow.ago(5, "minutes"))
            ),
            "p.received_at",
            "asc"
        )
    ),
    flow.result(
        {
            "device_id": flow.ref("input.device_id"),
            "status": flow.ref("status"),
            "positions": flow.ref("positions")
        }
    )
)

flow.http(
    "POST",
    "/api/orders",
    flow.input("user_id", flow.named("UserID")),
    flow.input("items", flow.named("Cart")),
    flow.input("payment_method", flow.named("PaymentMethod")),
    flow.input("email_failure", flow.named("Bool"), False),
    flow.transaction(
        flow.bind(
            "user",
            flow.one(flow.query("Users", "u", flow.equal(flow.ref("u.id"), flow.ref("input.user_id"))))
        ),
        flow.ensure(flow.present(flow.ref("user")), flow.error(404, "user_not_found", "Uživatel neexistuje.")),
        flow.bind("cart", flow.group_sum(flow.ref("input.items"), "product_id", "quantity")),
        flow.bind(
            "snapshots",
            flow.typed(
                flow.list_of(flow.named("OrderItem")),
                flow.map_each(
                    "item",
                    flow.ref("cart"),
                    flow.bind(
                        "product",
                        flow.one(
                            flow.query(
                                "Products",
                                "p",
                                flow.equal(flow.ref("p.id"), flow.ref("item.product_id"))
                            )
                        )
                    ),
                    flow.ensure(
                        flow.present(flow.ref("product")),
                        flow.error(404, "product_not_found", "Produkt neexistuje.")
                    ),
                    flow.result(
                        {
                            "id": flow.ref("product.id"),
                            "name": flow.ref("product.name"),
                            "price_cents": flow.ref("product.price_cents"),
                            "quantity": flow.ref("item.quantity")
                        }
                    )
                )
            )
        ),
        flow.bind(
            "total",
            flow.typed(
                flow.integer(),
                flow.sum(
                    "item",
                    flow.ref("snapshots"),
                    flow.multiply(flow.ref("item.price_cents"), flow.ref("item.quantity"))
                )
            )
        ),
        flow.bind(
            "order",
            flow.typed(
                flow.named("Order"),
                flow.insert(
                    "Orders",
                    {
                        "id": flow.uuid("ord"),
                        "user_id": flow.ref("user.id"),
                        "request_id": flow.ref("request.id"),
                        "created_at": flow.ref("request.time"),
                        "items": flow.ref("snapshots"),
                        "total_cents": flow.ref("total"),
                        "payment_method": flow.ref("input.payment_method"),
                        "payment_url": "",
                        "status": "awaiting_payment"
                    }
                )
            )
        ),
        flow.each(
            "item",
            flow.ref("snapshots"),
            flow.bind(
                "product",
                flow.one(flow.query("Products", "p", flow.equal(flow.ref("p.id"), flow.ref("item.id"))))
            ),
            flow.ensure(
                flow.present(flow.ref("product")),
                flow.error(404, "product_not_found", "Produkt neexistuje.")
            ),
            flow.ensure(
                flow.greater_equal(flow.ref("product.stock"), flow.ref("item.quantity")),
                flow.error(409, "insufficient_stock", "Nedostatek kusů na skladě.")
            ),
            flow.update(
                flow.query("Products", "p", flow.equal(flow.ref("p.id"), flow.ref("item.id"))),
                {"stock": flow.subtract(flow.ref("p.stock"), flow.ref("item.quantity"))}
            )
        ),
        flow.bind(
            "payment",
            flow.call(
                "Payment.create_url",
                {
                    "order_id": flow.ref("order.id"),
                    "amount_cents": flow.ref("order.total_cents"),
                    "currency": "CZK",
                    "country": flow.ref("user.country"),
                    "method": flow.ref("input.payment_method")
                }
            )
        ),
        flow.update(
            flow.query("Orders", "o", flow.equal(flow.ref("o.id"), flow.ref("order.id"))),
            {"payment_url": flow.ref("payment.url")}
        ),
        flow.queue(
            "Email.send_confirmation",
            {
                "order_id": flow.ref("order.id"),
                "to": flow.ref("user.email"),
                "subject": flow.concat(["Objednávka ", flow.ref("order.id")]),
                "payment_url": flow.ref("payment.url"),
                "total_cents": flow.ref("order.total_cents"),
                "items": flow.ref("snapshots"),
                "simulate_failure": flow.ref("input.email_failure")
            },
            flow.policy(flow.attempts(3), flow.delay(1000, "ms"), flow.retain())
        ),
        flow.commit()
    ),
    flow.response(
        201,
        {
            "order": flow.merge(flow.ref("order"), {"currency": "CZK", "payment_url": flow.ref("payment.url")}),
            "payment": flow.ref("payment"),
            "email": {"state": "queued"}
        }
    )
)

flow.http(
    "GET",
    "/api/orders/:order_id",
    flow.input("order_id", flow.named("String")),
    flow.bind(
        "order",
        flow.one(flow.query("Orders", "o", flow.equal(flow.ref("o.id"), flow.ref("input.order_id"))))
    ),
    flow.ensure(flow.present(flow.ref("order")), flow.error(404, "order_not_found", "Objednávka neexistuje.")),
    flow.result(flow.ref("order"))
)

flow.type(
    "ProductPhoto",
    flow.image(),
    flow.all(
        flow.less_equal(flow.ref("value.byte_size"), 10485760),
        flow.less_equal(flow.ref("value.width"), 8000),
        flow.less_equal(flow.ref("value.height"), 8000)
    )
)

flow.type("PhotoSize", flow.integer(), flow.between(flow.ref("value"), 1, 4096))

flow.table(
    "Photos",
    {
        "id": flow.string(),
        "product_id": flow.string(),
        "file_id": flow.string(),
        "url": flow.string(),
        "width": flow.integer(),
        "height": flow.integer()
    }
)

flow.http(
    "POST",
    "/api/products/:product_id/photo",
    flow.input("product_id", flow.named("String")),
    flow.input("photo", flow.named("ProductPhoto")),
    flow.input("width", flow.named("PhotoSize"), 800),
    flow.input("height", flow.named("PhotoSize"), 600),
    flow.transaction(
        flow.bind(
            "product",
            flow.one(flow.query("Products", "p", flow.equal(flow.ref("p.id"), flow.ref("input.product_id"))))
        ),
        flow.ensure(
            flow.present(flow.ref("product")),
            flow.error(404, "product_not_found", "Produkt neexistuje.")
        ),
        flow.bind(
            "resized",
            flow.typed(
                flow.named("ProductPhoto"),
                flow.call(
                    "Image.resize",
                    {
                        "image": flow.ref("input.photo"),
                        "width": flow.ref("input.width"),
                        "height": flow.ref("input.height")
                    }
                )
            )
        ),
        flow.bind("stored", flow.call("Files.put", {"file": flow.ref("resized")})),
        flow.bind(
            "saved",
            flow.insert(
                "Photos",
                {
                    "id": flow.uuid("photo"),
                    "product_id": flow.ref("input.product_id"),
                    "file_id": flow.ref("stored.id"),
                    "url": flow.ref("stored.url"),
                    "width": flow.ref("resized.width"),
                    "height": flow.ref("resized.height")
                }
            )
        ),
        flow.commit()
    ),
    flow.response(201, {"photo": flow.ref("saved"), "file": flow.ref("stored")})
)

flow.http(
    "GET",
    "/api/products/:product_id/photos",
    flow.input("product_id", flow.named("String")),
    flow.bind(
        "photos",
        flow.query("Photos", "p", flow.equal(flow.ref("p.product_id"), flow.ref("input.product_id")))
    ),
    flow.result({"photos": flow.ref("photos")})
)

flow.table(
    "DeviceAccess",
    {
        "id": flow.string(),
        "user_id": flow.named("UserID"),
        "token": flow.string(),
        "device_id": flow.named("DeviceID")
    }
)

flow.seed(
    "DeviceAccess",
    [
        {"id": "petra-mower1", "user_id": "u1", "token": "demo-petra", "device_id": "mower1"},
        {"id": "david-mower2", "user_id": "u2", "token": "demo-david", "device_id": "mower2"}
    ]
)

flow.websocket(
    "/ws",
    flow.input("token", flow.named("String")),
    flow.authorize(
        flow.exists(
            flow.query(
                "DeviceAccess",
                "access",
                flow.all(
                    flow.equal(flow.ref("access.token"), flow.ref("input.token")),
                    flow.exists(
                        flow.query("Users", "user", flow.equal(flow.ref("user.id"), flow.ref("access.user_id")))
                    )
                )
            )
        )
    ),
    flow.allow_source(
        "DeviceStatus",
        flow.exists(
            flow.query(
                "DeviceAccess",
                "access",
                flow.all(
                    flow.equal(flow.ref("access.token"), flow.ref("input.token")),
                    flow.equal(flow.ref("access.device_id"), flow.ref("input.device_id"))
                )
            )
        )
    ),
    flow.allow_source(
        "DevicePosition",
        flow.exists(
            flow.query(
                "DeviceAccess",
                "access",
                flow.all(
                    flow.equal(flow.ref("access.token"), flow.ref("input.token")),
                    flow.equal(flow.ref("access.device_id"), flow.ref("input.device_id"))
                )
            )
        )
    )
)

flow.type("MowerAction", flow.string(), flow.member(flow.ref("value"), ["start", "stop"]))

flow.mqtt(
    "DeviceCommand",
    flow.topic("devices/{device_id}/command"),
    flow.input("device_id", flow.named("DeviceID")),
    flow.payload({"command_id": flow.string(), "action": flow.named("MowerAction")}),
    flow.history(24, "hours")
)

flow.http(
    "POST",
    "/api/devices/:device_id/commands",
    flow.input("device_id", flow.named("DeviceID")),
    flow.input("action", flow.named("MowerAction")),
    flow.input("token", flow.named("String")),
    flow.ensure(
        flow.exists(
            flow.query(
                "DeviceAccess",
                "access",
                flow.all(
                    flow.equal(flow.ref("access.token"), flow.ref("input.token")),
                    flow.equal(flow.ref("access.device_id"), flow.ref("input.device_id")),
                    flow.exists(
                        flow.query("Users", "user", flow.equal(flow.ref("user.id"), flow.ref("access.user_id")))
                    )
                )
            )
        ),
        flow.error(403, "forbidden", "Přístup k zařízení je zamítnut.")
    ),
    flow.transaction(
        flow.bind(
            "outgoing",
            flow.publish(
                flow.source("DeviceCommand", flow.ref("input.device_id")),
                {"command_id": flow.uuid("command"), "action": flow.ref("input.action")}
            )
        ),
        flow.commit()
    ),
    flow.response(202, flow.ref("outgoing"))
)
