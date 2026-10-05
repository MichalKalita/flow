fn main() {
    // Přepis prototype/priv/workflows/application.flow; návrhové API Flow není implementované.
    // bind/ref označují symboly scénáře; vnořené kroky se deklarují, nevykonávají při registraci.
    // Zachovává pravidla současného Flow včetně jeho neomezených dotazů a obecných číselných typů.

    flow::r#type(
        "UserID",
        flow::string(),
        flow::greater(flow::length(flow::r#ref("value")), 0),
    );

    flow::r#type(
        "Quantity",
        flow::integer(),
        flow::between(flow::r#ref("value"), 1, 100),
    );

    flow::r#type(
        "Stock",
        flow::integer(),
        flow::greater_equal(flow::r#ref("value"), 0),
    );

    flow::r#type(
        "Price",
        flow::integer(),
        flow::greater_equal(flow::r#ref("value"), 0),
    );

    flow::r#type(
        "CartItem",
        flow::record([
            ("product_id", flow::string()),
            ("quantity", flow::named("Quantity")),
        ]),
    );

    flow::r#type(
        "Cart",
        flow::list_of(flow::named("CartItem")),
        flow::between(flow::length(flow::r#ref("value")), 1, 50),
    );

    flow::r#type(
        "PaymentMethod",
        flow::string(),
        flow::member(flow::r#ref("value"), vec!["card", "bank"]),
    );

    flow::r#type(
        "Product",
        flow::record([
            ("id", flow::string()),
            ("name", flow::string()),
            ("price_cents", flow::named("Price")),
            ("stock", flow::named("Stock")),
        ]),
    );

    flow::r#type(
        "User",
        flow::record([
            ("id", flow::string()),
            ("name", flow::string()),
            ("email", flow::string()),
            ("country", flow::string()),
        ]),
    );

    flow::r#type(
        "OrderItem",
        flow::record([
            ("id", flow::string()),
            ("name", flow::string()),
            ("price_cents", flow::named("Price")),
            ("quantity", flow::integer()),
        ]),
    );

    flow::r#type(
        "Order",
        flow::record([
            ("id", flow::string()),
            ("user_id", flow::string()),
            ("request_id", flow::string()),
            ("created_at", flow::string()),
            ("items", flow::list_of(flow::named("OrderItem"))),
            ("total_cents", flow::integer()),
            ("payment_method", flow::named("PaymentMethod")),
            ("payment_url", flow::string()),
            ("status", flow::string()),
        ]),
    );

    flow::table("Products", flow::named("Product"));

    flow::table("Users", flow::named("User"));

    flow::table("Orders", flow::named("Order"));

    flow::seed(
        "Products",
        vec![
            flow::record([
                ("id", "p1"),
                ("name", "Studio sluchátka"),
                ("price_cents", 249000),
                ("stock", 12),
            ]),
            flow::record([
                ("id", "p2"),
                ("name", "Mechanická klávesnice"),
                ("price_cents", 329000),
                ("stock", 5),
            ]),
            flow::record([
                ("id", "p3"),
                ("name", "USB-C rozbočovač"),
                ("price_cents", 129000),
                ("stock", 2),
            ]),
            flow::record([
                ("id", "p4"),
                ("name", "Webkamera"),
                ("price_cents", 189000),
                ("stock", 0),
            ]),
        ],
    );

    flow::seed(
        "Users",
        vec![
            flow::record([
                ("id", "u1"),
                ("name", "Petra Nováková"),
                ("email", "petra@example.test"),
                ("country", "CZ"),
            ]),
            flow::record([
                ("id", "u2"),
                ("name", "David Miller"),
                ("email", "david@example.test"),
                ("country", "US"),
            ]),
            flow::record([
                ("id", "u3"),
                ("name", "Nora Silva"),
                ("email", "nora@example.test"),
                ("country", "BR"),
            ]),
        ],
    );

    flow::r#type(
        "DeviceID",
        flow::string(),
        flow::between(flow::length(flow::r#ref("value")), 1, 64),
    );

    flow::r#type(
        "Battery",
        flow::integer(),
        flow::between(flow::r#ref("value"), 0, 100),
    );

    flow::r#type(
        "Latitude",
        flow::number(),
        flow::between(flow::r#ref("value"), -90, 90),
    );

    flow::r#type(
        "Longitude",
        flow::number(),
        flow::between(flow::r#ref("value"), -180, 180),
    );

    flow::mqtt(
        "DeviceStatus",
        flow::topic("devices/{device_id}/status"),
        flow::input("device_id", flow::named("DeviceID")),
        flow::payload(flow::record([
            ("online", flow::boolean()),
            ("battery", flow::named("Battery")),
        ])),
        flow::history(24, "hours"),
    );

    flow::mqtt(
        "DevicePosition",
        flow::topic("devices/{device_id}/position"),
        flow::input("device_id", flow::named("DeviceID")),
        flow::payload(flow::record([
            ("latitude", flow::named("Latitude")),
            ("longitude", flow::named("Longitude")),
        ])),
        flow::history(5, "minutes"),
    );

    flow::table(
        "DeviceAlerts",
        flow::record([
            ("id", flow::string()),
            ("device_id", flow::named("DeviceID")),
            ("kind", flow::string()),
            ("created_at", flow::string()),
        ]),
    );

    flow::on_mqtt(
        "DeviceStatus",
        flow::when(
            flow::less(flow::r#ref("message.battery"), 20),
            flow::transaction(
                flow::insert(
                    "DeviceAlerts",
                    flow::record([
                        ("id", flow::uuid("alert")),
                        ("device_id", flow::r#ref("input.device_id")),
                        ("kind", "low_battery"),
                        ("created_at", flow::r#ref("request.time")),
                    ]),
                ),
                flow::commit(),
            ),
        ),
        flow::result(flow::record([("accepted", true)])),
    );

    flow::http(
        "GET",
        "/api/device-alerts/:device_id",
        flow::input("device_id", flow::named("DeviceID")),
        flow::bind(
            "alerts",
            flow::query(
                "DeviceAlerts",
                "a",
                flow::equal(flow::r#ref("a.device_id"), flow::r#ref("input.device_id")),
            ),
        ),
        flow::result(flow::record([("alerts", flow::r#ref("alerts"))])),
    );

    flow::http(
        "GET",
        "/api/devices/:device_id",
        flow::input("device_id", flow::named("DeviceID")),
        flow::bind(
            "status",
            flow::last(flow::source("DeviceStatus", flow::r#ref("input.device_id"))),
        ),
        flow::bind(
            "positions",
            flow::order_by(
                flow::query_source(
                    "DevicePosition",
                    flow::r#ref("input.device_id"),
                    "p",
                    flow::greater_equal(flow::r#ref("p.received_at"), flow::ago(5, "minutes")),
                ),
                "p.received_at",
                "asc",
            ),
        ),
        flow::result(flow::record([
            ("device_id", flow::r#ref("input.device_id")),
            ("status", flow::r#ref("status")),
            ("positions", flow::r#ref("positions")),
        ])),
    );

    flow::http(
        "POST",
        "/api/orders",
        flow::input("user_id", flow::named("UserID")),
        flow::input("items", flow::named("Cart")),
        flow::input("payment_method", flow::named("PaymentMethod")),
        flow::input("email_failure", flow::named("Bool"), false),
        flow::transaction(
            flow::bind(
                "user",
                flow::one(flow::query(
                    "Users",
                    "u",
                    flow::equal(flow::r#ref("u.id"), flow::r#ref("input.user_id")),
                )),
            ),
            flow::ensure(
                flow::present(flow::r#ref("user")),
                flow::error(404, "user_not_found", "Uživatel neexistuje."),
            ),
            flow::bind(
                "cart",
                flow::group_sum(flow::r#ref("input.items"), "product_id", "quantity"),
            ),
            flow::bind(
                "snapshots",
                flow::typed(
                    flow::list_of(flow::named("OrderItem")),
                    flow::map_each(
                        "item",
                        flow::r#ref("cart"),
                        flow::bind(
                            "product",
                            flow::one(flow::query(
                                "Products",
                                "p",
                                flow::equal(flow::r#ref("p.id"), flow::r#ref("item.product_id")),
                            )),
                        ),
                        flow::ensure(
                            flow::present(flow::r#ref("product")),
                            flow::error(404, "product_not_found", "Produkt neexistuje."),
                        ),
                        flow::result(flow::record([
                            ("id", flow::r#ref("product.id")),
                            ("name", flow::r#ref("product.name")),
                            ("price_cents", flow::r#ref("product.price_cents")),
                            ("quantity", flow::r#ref("item.quantity")),
                        ])),
                    ),
                ),
            ),
            flow::bind(
                "total",
                flow::typed(
                    flow::integer(),
                    flow::sum(
                        "item",
                        flow::r#ref("snapshots"),
                        flow::multiply(
                            flow::r#ref("item.price_cents"),
                            flow::r#ref("item.quantity"),
                        ),
                    ),
                ),
            ),
            flow::bind(
                "order",
                flow::typed(
                    flow::named("Order"),
                    flow::insert(
                        "Orders",
                        flow::record([
                            ("id", flow::uuid("ord")),
                            ("user_id", flow::r#ref("user.id")),
                            ("request_id", flow::r#ref("request.id")),
                            ("created_at", flow::r#ref("request.time")),
                            ("items", flow::r#ref("snapshots")),
                            ("total_cents", flow::r#ref("total")),
                            ("payment_method", flow::r#ref("input.payment_method")),
                            ("payment_url", ""),
                            ("status", "awaiting_payment"),
                        ]),
                    ),
                ),
            ),
            flow::each(
                "item",
                flow::r#ref("snapshots"),
                flow::bind(
                    "product",
                    flow::one(flow::query(
                        "Products",
                        "p",
                        flow::equal(flow::r#ref("p.id"), flow::r#ref("item.id")),
                    )),
                ),
                flow::ensure(
                    flow::present(flow::r#ref("product")),
                    flow::error(404, "product_not_found", "Produkt neexistuje."),
                ),
                flow::ensure(
                    flow::greater_equal(flow::r#ref("product.stock"), flow::r#ref("item.quantity")),
                    flow::error(409, "insufficient_stock", "Nedostatek kusů na skladě."),
                ),
                flow::update(
                    flow::query(
                        "Products",
                        "p",
                        flow::equal(flow::r#ref("p.id"), flow::r#ref("item.id")),
                    ),
                    flow::record([(
                        "stock",
                        flow::subtract(flow::r#ref("p.stock"), flow::r#ref("item.quantity")),
                    )]),
                ),
            ),
            flow::bind(
                "payment",
                flow::call(
                    "Payment.create_url",
                    flow::record([
                        ("order_id", flow::r#ref("order.id")),
                        ("amount_cents", flow::r#ref("order.total_cents")),
                        ("currency", "CZK"),
                        ("country", flow::r#ref("user.country")),
                        ("method", flow::r#ref("input.payment_method")),
                    ]),
                ),
            ),
            flow::update(
                flow::query(
                    "Orders",
                    "o",
                    flow::equal(flow::r#ref("o.id"), flow::r#ref("order.id")),
                ),
                flow::record([("payment_url", flow::r#ref("payment.url"))]),
            ),
            flow::queue(
                "Email.send_confirmation",
                flow::record([
                    ("order_id", flow::r#ref("order.id")),
                    ("to", flow::r#ref("user.email")),
                    (
                        "subject",
                        flow::concat(vec!["Objednávka ", flow::r#ref("order.id")]),
                    ),
                    ("payment_url", flow::r#ref("payment.url")),
                    ("total_cents", flow::r#ref("order.total_cents")),
                    ("items", flow::r#ref("snapshots")),
                    ("simulate_failure", flow::r#ref("input.email_failure")),
                ]),
                flow::policy(flow::attempts(3), flow::delay(1000, "ms"), flow::retain()),
            ),
            flow::commit(),
        ),
        flow::response(
            201,
            flow::record([
                (
                    "order",
                    flow::merge(
                        flow::r#ref("order"),
                        flow::record([
                            ("currency", "CZK"),
                            ("payment_url", flow::r#ref("payment.url")),
                        ]),
                    ),
                ),
                ("payment", flow::r#ref("payment")),
                ("email", flow::record([("state", "queued")])),
            ]),
        ),
    );

    flow::http(
        "GET",
        "/api/orders/:order_id",
        flow::input("order_id", flow::named("String")),
        flow::bind(
            "order",
            flow::one(flow::query(
                "Orders",
                "o",
                flow::equal(flow::r#ref("o.id"), flow::r#ref("input.order_id")),
            )),
        ),
        flow::ensure(
            flow::present(flow::r#ref("order")),
            flow::error(404, "order_not_found", "Objednávka neexistuje."),
        ),
        flow::result(flow::r#ref("order")),
    );

    flow::r#type(
        "ProductPhoto",
        flow::image(),
        flow::all(
            flow::less_equal(flow::r#ref("value.byte_size"), 10485760),
            flow::less_equal(flow::r#ref("value.width"), 8000),
            flow::less_equal(flow::r#ref("value.height"), 8000),
        ),
    );

    flow::r#type(
        "PhotoSize",
        flow::integer(),
        flow::between(flow::r#ref("value"), 1, 4096),
    );

    flow::table(
        "Photos",
        flow::record([
            ("id", flow::string()),
            ("product_id", flow::string()),
            ("file_id", flow::string()),
            ("url", flow::string()),
            ("width", flow::integer()),
            ("height", flow::integer()),
        ]),
    );

    flow::http(
        "POST",
        "/api/products/:product_id/photo",
        flow::input("product_id", flow::named("String")),
        flow::input("photo", flow::named("ProductPhoto")),
        flow::input("width", flow::named("PhotoSize"), 800),
        flow::input("height", flow::named("PhotoSize"), 600),
        flow::transaction(
            flow::bind(
                "product",
                flow::one(flow::query(
                    "Products",
                    "p",
                    flow::equal(flow::r#ref("p.id"), flow::r#ref("input.product_id")),
                )),
            ),
            flow::ensure(
                flow::present(flow::r#ref("product")),
                flow::error(404, "product_not_found", "Produkt neexistuje."),
            ),
            flow::bind(
                "resized",
                flow::typed(
                    flow::named("ProductPhoto"),
                    flow::call(
                        "Image.resize",
                        flow::record([
                            ("image", flow::r#ref("input.photo")),
                            ("width", flow::r#ref("input.width")),
                            ("height", flow::r#ref("input.height")),
                        ]),
                    ),
                ),
            ),
            flow::bind(
                "stored",
                flow::call(
                    "Files.put",
                    flow::record([("file", flow::r#ref("resized"))]),
                ),
            ),
            flow::bind(
                "saved",
                flow::insert(
                    "Photos",
                    flow::record([
                        ("id", flow::uuid("photo")),
                        ("product_id", flow::r#ref("input.product_id")),
                        ("file_id", flow::r#ref("stored.id")),
                        ("url", flow::r#ref("stored.url")),
                        ("width", flow::r#ref("resized.width")),
                        ("height", flow::r#ref("resized.height")),
                    ]),
                ),
            ),
            flow::commit(),
        ),
        flow::response(
            201,
            flow::record([
                ("photo", flow::r#ref("saved")),
                ("file", flow::r#ref("stored")),
            ]),
        ),
    );

    flow::http(
        "GET",
        "/api/products/:product_id/photos",
        flow::input("product_id", flow::named("String")),
        flow::bind(
            "photos",
            flow::query(
                "Photos",
                "p",
                flow::equal(flow::r#ref("p.product_id"), flow::r#ref("input.product_id")),
            ),
        ),
        flow::result(flow::record([("photos", flow::r#ref("photos"))])),
    );

    flow::table(
        "DeviceAccess",
        flow::record([
            ("id", flow::string()),
            ("user_id", flow::named("UserID")),
            ("token", flow::string()),
            ("device_id", flow::named("DeviceID")),
        ]),
    );

    flow::seed(
        "DeviceAccess",
        vec![
            flow::record([
                ("id", "petra-mower1"),
                ("user_id", "u1"),
                ("token", "demo-petra"),
                ("device_id", "mower1"),
            ]),
            flow::record([
                ("id", "david-mower2"),
                ("user_id", "u2"),
                ("token", "demo-david"),
                ("device_id", "mower2"),
            ]),
        ],
    );

    flow::websocket(
        "/ws",
        flow::input("token", flow::named("String")),
        flow::authorize(flow::exists(flow::query(
            "DeviceAccess",
            "access",
            flow::all(
                flow::equal(flow::r#ref("access.token"), flow::r#ref("input.token")),
                flow::exists(flow::query(
                    "Users",
                    "user",
                    flow::equal(flow::r#ref("user.id"), flow::r#ref("access.user_id")),
                )),
            ),
        ))),
        flow::allow_source(
            "DeviceStatus",
            flow::exists(flow::query(
                "DeviceAccess",
                "access",
                flow::all(
                    flow::equal(flow::r#ref("access.token"), flow::r#ref("input.token")),
                    flow::equal(
                        flow::r#ref("access.device_id"),
                        flow::r#ref("input.device_id"),
                    ),
                ),
            )),
        ),
        flow::allow_source(
            "DevicePosition",
            flow::exists(flow::query(
                "DeviceAccess",
                "access",
                flow::all(
                    flow::equal(flow::r#ref("access.token"), flow::r#ref("input.token")),
                    flow::equal(
                        flow::r#ref("access.device_id"),
                        flow::r#ref("input.device_id"),
                    ),
                ),
            )),
        ),
    );

    flow::r#type(
        "MowerAction",
        flow::string(),
        flow::member(flow::r#ref("value"), vec!["start", "stop"]),
    );

    flow::mqtt(
        "DeviceCommand",
        flow::topic("devices/{device_id}/command"),
        flow::input("device_id", flow::named("DeviceID")),
        flow::payload(flow::record([
            ("command_id", flow::string()),
            ("action", flow::named("MowerAction")),
        ])),
        flow::history(24, "hours"),
    );

    flow::http(
        "POST",
        "/api/devices/:device_id/commands",
        flow::input("device_id", flow::named("DeviceID")),
        flow::input("action", flow::named("MowerAction")),
        flow::input("token", flow::named("String")),
        flow::ensure(
            flow::exists(flow::query(
                "DeviceAccess",
                "access",
                flow::all(
                    flow::equal(flow::r#ref("access.token"), flow::r#ref("input.token")),
                    flow::equal(
                        flow::r#ref("access.device_id"),
                        flow::r#ref("input.device_id"),
                    ),
                    flow::exists(flow::query(
                        "Users",
                        "user",
                        flow::equal(flow::r#ref("user.id"), flow::r#ref("access.user_id")),
                    )),
                ),
            )),
            flow::error(403, "forbidden", "Přístup k zařízení je zamítnut."),
        ),
        flow::transaction(
            flow::bind(
                "outgoing",
                flow::publish(
                    flow::source("DeviceCommand", flow::r#ref("input.device_id")),
                    flow::record([
                        ("command_id", flow::uuid("command")),
                        ("action", flow::r#ref("input.action")),
                    ]),
                ),
            ),
            flow::commit(),
        ),
        flow::response(202, flow::r#ref("outgoing")),
    );
}
