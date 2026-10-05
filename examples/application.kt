fun main() {
// Přepis prototype/priv/workflows/application.flow; návrhové API Flow není implementované.
// bind/ref označují symboly scénáře; vnořené kroky se deklarují, nevykonávají při registraci.
// Zachovává pravidla současného Flow včetně jeho neomezených dotazů a obecných číselných typů.

    Flow.type("UserID", Flow.string(), Flow.greater(Flow.length(Flow.ref("value")), 0))

    Flow.type("Quantity", Flow.integer(), Flow.between(Flow.ref("value"), 1, 100))

    Flow.type("Stock", Flow.integer(), Flow.greaterEqual(Flow.ref("value"), 0))

    Flow.type("Price", Flow.integer(), Flow.greaterEqual(Flow.ref("value"), 0))

    Flow.type("CartItem", mapOf("product_id" to Flow.string(), "quantity" to Flow.named("Quantity")))

    Flow.type("Cart", Flow.listOf(Flow.named("CartItem")), Flow.between(Flow.length(Flow.ref("value")), 1, 50))

    Flow.type("PaymentMethod", Flow.string(), Flow.member(Flow.ref("value"), listOf("card", "bank")))

    Flow.type(
        "Product",
        mapOf(
            "id" to Flow.string(),
            "name" to Flow.string(),
            "price_cents" to Flow.named("Price"),
            "stock" to Flow.named("Stock")
        )
    )

    Flow.type(
        "User",
        mapOf(
            "id" to Flow.string(),
            "name" to Flow.string(),
            "email" to Flow.string(),
            "country" to Flow.string()
        )
    )

    Flow.type(
        "OrderItem",
        mapOf(
            "id" to Flow.string(),
            "name" to Flow.string(),
            "price_cents" to Flow.named("Price"),
            "quantity" to Flow.integer()
        )
    )

    Flow.type(
        "Order",
        mapOf(
            "id" to Flow.string(),
            "user_id" to Flow.string(),
            "request_id" to Flow.string(),
            "created_at" to Flow.string(),
            "items" to Flow.listOf(Flow.named("OrderItem")),
            "total_cents" to Flow.integer(),
            "payment_method" to Flow.named("PaymentMethod"),
            "payment_url" to Flow.string(),
            "status" to Flow.string()
        )
    )

    Flow.table("Products", Flow.named("Product"))

    Flow.table("Users", Flow.named("User"))

    Flow.table("Orders", Flow.named("Order"))

    Flow.seed(
        "Products",
        listOf(
            mapOf("id" to "p1", "name" to "Studio sluchátka", "price_cents" to 249000, "stock" to 12),
            mapOf("id" to "p2", "name" to "Mechanická klávesnice", "price_cents" to 329000, "stock" to 5),
            mapOf("id" to "p3", "name" to "USB-C rozbočovač", "price_cents" to 129000, "stock" to 2),
            mapOf("id" to "p4", "name" to "Webkamera", "price_cents" to 189000, "stock" to 0)
        )
    )

    Flow.seed(
        "Users",
        listOf(
            mapOf("id" to "u1", "name" to "Petra Nováková", "email" to "petra@example.test", "country" to "CZ"),
            mapOf("id" to "u2", "name" to "David Miller", "email" to "david@example.test", "country" to "US"),
            mapOf("id" to "u3", "name" to "Nora Silva", "email" to "nora@example.test", "country" to "BR")
        )
    )

    Flow.type("DeviceID", Flow.string(), Flow.between(Flow.length(Flow.ref("value")), 1, 64))

    Flow.type("Battery", Flow.integer(), Flow.between(Flow.ref("value"), 0, 100))

    Flow.type("Latitude", Flow.number(), Flow.between(Flow.ref("value"), -90, 90))

    Flow.type("Longitude", Flow.number(), Flow.between(Flow.ref("value"), -180, 180))

    Flow.mqtt(
        "DeviceStatus",
        Flow.topic("devices/{device_id}/status"),
        Flow.input("device_id", Flow.named("DeviceID")),
        Flow.payload(mapOf("online" to Flow.boolean(), "battery" to Flow.named("Battery"))),
        Flow.history(24, "hours")
    )

    Flow.mqtt(
        "DevicePosition",
        Flow.topic("devices/{device_id}/position"),
        Flow.input("device_id", Flow.named("DeviceID")),
        Flow.payload(mapOf("latitude" to Flow.named("Latitude"), "longitude" to Flow.named("Longitude"))),
        Flow.history(5, "minutes")
    )

    Flow.table(
        "DeviceAlerts",
        mapOf(
            "id" to Flow.string(),
            "device_id" to Flow.named("DeviceID"),
            "kind" to Flow.string(),
            "created_at" to Flow.string()
        )
    )

    Flow.onMqtt(
        "DeviceStatus",
        Flow.`when`(
            Flow.less(Flow.ref("message.battery"), 20),
            Flow.transaction(
                Flow.insert(
                    "DeviceAlerts",
                    mapOf(
                        "id" to Flow.uuid("alert"),
                        "device_id" to Flow.ref("input.device_id"),
                        "kind" to "low_battery",
                        "created_at" to Flow.ref("request.time")
                    )
                ),
                Flow.commit()
            )
        ),
        Flow.result(mapOf("accepted" to true))
    )

    Flow.http(
        "GET",
        "/api/device-alerts/:device_id",
        Flow.input("device_id", Flow.named("DeviceID")),
        Flow.bind(
            "alerts",
            Flow.query("DeviceAlerts", "a", Flow.equal(Flow.ref("a.device_id"), Flow.ref("input.device_id")))
        ),
        Flow.result(mapOf("alerts" to Flow.ref("alerts")))
    )

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
            mapOf(
                "device_id" to Flow.ref("input.device_id"),
                "status" to Flow.ref("status"),
                "positions" to Flow.ref("positions")
            )
        )
    )

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
            Flow.ensure(
                Flow.present(Flow.ref("user")),
                Flow.error(404, "user_not_found", "Uživatel neexistuje.")
            ),
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
                            mapOf(
                                "id" to Flow.ref("product.id"),
                                "name" to Flow.ref("product.name"),
                                "price_cents" to Flow.ref("product.price_cents"),
                                "quantity" to Flow.ref("item.quantity")
                            )
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
                        mapOf(
                            "id" to Flow.uuid("ord"),
                            "user_id" to Flow.ref("user.id"),
                            "request_id" to Flow.ref("request.id"),
                            "created_at" to Flow.ref("request.time"),
                            "items" to Flow.ref("snapshots"),
                            "total_cents" to Flow.ref("total"),
                            "payment_method" to Flow.ref("input.payment_method"),
                            "payment_url" to "",
                            "status" to "awaiting_payment"
                        )
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
                    mapOf("stock" to Flow.subtract(Flow.ref("p.stock"), Flow.ref("item.quantity")))
                )
            ),
            Flow.bind(
                "payment",
                Flow.call(
                    "Payment.create_url",
                    mapOf(
                        "order_id" to Flow.ref("order.id"),
                        "amount_cents" to Flow.ref("order.total_cents"),
                        "currency" to "CZK",
                        "country" to Flow.ref("user.country"),
                        "method" to Flow.ref("input.payment_method")
                    )
                )
            ),
            Flow.update(
                Flow.query("Orders", "o", Flow.equal(Flow.ref("o.id"), Flow.ref("order.id"))),
                mapOf("payment_url" to Flow.ref("payment.url"))
            ),
            Flow.queue(
                "Email.send_confirmation",
                mapOf(
                    "order_id" to Flow.ref("order.id"),
                    "to" to Flow.ref("user.email"),
                    "subject" to Flow.concat(listOf("Objednávka ", Flow.ref("order.id"))),
                    "payment_url" to Flow.ref("payment.url"),
                    "total_cents" to Flow.ref("order.total_cents"),
                    "items" to Flow.ref("snapshots"),
                    "simulate_failure" to Flow.ref("input.email_failure")
                ),
                Flow.policy(Flow.attempts(3), Flow.delay(1000, "ms"), Flow.retain())
            ),
            Flow.commit()
        ),
        Flow.response(
            201,
            mapOf(
                "order" to Flow.merge(
                    Flow.ref("order"),
                    mapOf("currency" to "CZK", "payment_url" to Flow.ref("payment.url"))
                ),
                "payment" to Flow.ref("payment"),
                "email" to mapOf("state" to "queued")
            )
        )
    )

    Flow.http(
        "GET",
        "/api/orders/:order_id",
        Flow.input("order_id", Flow.named("String")),
        Flow.bind(
            "order",
            Flow.one(Flow.query("Orders", "o", Flow.equal(Flow.ref("o.id"), Flow.ref("input.order_id"))))
        ),
        Flow.ensure(
            Flow.present(Flow.ref("order")),
            Flow.error(404, "order_not_found", "Objednávka neexistuje.")
        ),
        Flow.result(Flow.ref("order"))
    )

    Flow.type(
        "ProductPhoto",
        Flow.image(),
        Flow.all(
            Flow.lessEqual(Flow.ref("value.byte_size"), 10485760),
            Flow.lessEqual(Flow.ref("value.width"), 8000),
            Flow.lessEqual(Flow.ref("value.height"), 8000)
        )
    )

    Flow.type("PhotoSize", Flow.integer(), Flow.between(Flow.ref("value"), 1, 4096))

    Flow.table(
        "Photos",
        mapOf(
            "id" to Flow.string(),
            "product_id" to Flow.string(),
            "file_id" to Flow.string(),
            "url" to Flow.string(),
            "width" to Flow.integer(),
            "height" to Flow.integer()
        )
    )

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
                Flow.one(
                    Flow.query("Products", "p", Flow.equal(Flow.ref("p.id"), Flow.ref("input.product_id")))
                )
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
                        mapOf(
                            "image" to Flow.ref("input.photo"),
                            "width" to Flow.ref("input.width"),
                            "height" to Flow.ref("input.height")
                        )
                    )
                )
            ),
            Flow.bind("stored", Flow.call("Files.put", mapOf("file" to Flow.ref("resized")))),
            Flow.bind(
                "saved",
                Flow.insert(
                    "Photos",
                    mapOf(
                        "id" to Flow.uuid("photo"),
                        "product_id" to Flow.ref("input.product_id"),
                        "file_id" to Flow.ref("stored.id"),
                        "url" to Flow.ref("stored.url"),
                        "width" to Flow.ref("resized.width"),
                        "height" to Flow.ref("resized.height")
                    )
                )
            ),
            Flow.commit()
        ),
        Flow.response(201, mapOf("photo" to Flow.ref("saved"), "file" to Flow.ref("stored")))
    )

    Flow.http(
        "GET",
        "/api/products/:product_id/photos",
        Flow.input("product_id", Flow.named("String")),
        Flow.bind(
            "photos",
            Flow.query("Photos", "p", Flow.equal(Flow.ref("p.product_id"), Flow.ref("input.product_id")))
        ),
        Flow.result(mapOf("photos" to Flow.ref("photos")))
    )

    Flow.table(
        "DeviceAccess",
        mapOf(
            "id" to Flow.string(),
            "user_id" to Flow.named("UserID"),
            "token" to Flow.string(),
            "device_id" to Flow.named("DeviceID")
        )
    )

    Flow.seed(
        "DeviceAccess",
        listOf(
            mapOf("id" to "petra-mower1", "user_id" to "u1", "token" to "demo-petra", "device_id" to "mower1"),
            mapOf("id" to "david-mower2", "user_id" to "u2", "token" to "demo-david", "device_id" to "mower2")
        )
    )

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
                            Flow.query(
                                "Users",
                                "user",
                                Flow.equal(Flow.ref("user.id"), Flow.ref("access.user_id"))
                            )
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
    )

    Flow.type("MowerAction", Flow.string(), Flow.member(Flow.ref("value"), listOf("start", "stop")))

    Flow.mqtt(
        "DeviceCommand",
        Flow.topic("devices/{device_id}/command"),
        Flow.input("device_id", Flow.named("DeviceID")),
        Flow.payload(mapOf("command_id" to Flow.string(), "action" to Flow.named("MowerAction"))),
        Flow.history(24, "hours")
    )

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
                            Flow.query(
                                "Users",
                                "user",
                                Flow.equal(Flow.ref("user.id"), Flow.ref("access.user_id"))
                            )
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
                    mapOf("command_id" to Flow.uuid("command"), "action" to Flow.ref("input.action"))
                )
            ),
            Flow.commit()
        ),
        Flow.response(202, Flow.ref("outgoing"))
    )
}
