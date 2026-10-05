package main

func main() {
	// Přepis prototype/priv/workflows/application.flow; návrhové API Flow není implementované.
	// bind/ref označují symboly scénáře; vnořené kroky se deklarují, nevykonávají při registraci.
	// Zachovává pravidla současného Flow včetně jeho neomezených dotazů a obecných číselných typů.

	flow.Type("UserID", flow.String(), flow.Greater(flow.Length(flow.Ref("value")), 0))

	flow.Type("Quantity", flow.Integer(), flow.Between(flow.Ref("value"), 1, 100))

	flow.Type("Stock", flow.Integer(), flow.GreaterEqual(flow.Ref("value"), 0))

	flow.Type("Price", flow.Integer(), flow.GreaterEqual(flow.Ref("value"), 0))

	flow.Type("CartItem", map[string]any{"product_id": flow.String(), "quantity": flow.Named("Quantity")})

	flow.Type("Cart", flow.ListOf(flow.Named("CartItem")), flow.Between(flow.Length(flow.Ref("value")), 1, 50))

	flow.Type("PaymentMethod", flow.String(), flow.Member(flow.Ref("value"), []any{"card", "bank"}))

	flow.Type(
		"Product",
		map[string]any{
			"id":          flow.String(),
			"name":        flow.String(),
			"price_cents": flow.Named("Price"),
			"stock":       flow.Named("Stock"),
		},
	)

	flow.Type(
		"User",
		map[string]any{
			"id":      flow.String(),
			"name":    flow.String(),
			"email":   flow.String(),
			"country": flow.String(),
		},
	)

	flow.Type(
		"OrderItem",
		map[string]any{
			"id":          flow.String(),
			"name":        flow.String(),
			"price_cents": flow.Named("Price"),
			"quantity":    flow.Integer(),
		},
	)

	flow.Type(
		"Order",
		map[string]any{
			"id":             flow.String(),
			"user_id":        flow.String(),
			"request_id":     flow.String(),
			"created_at":     flow.String(),
			"items":          flow.ListOf(flow.Named("OrderItem")),
			"total_cents":    flow.Integer(),
			"payment_method": flow.Named("PaymentMethod"),
			"payment_url":    flow.String(),
			"status":         flow.String(),
		},
	)

	flow.Table("Products", flow.Named("Product"))

	flow.Table("Users", flow.Named("User"))

	flow.Table("Orders", flow.Named("Order"))

	flow.Seed(
		"Products",
		[]any{
			map[string]any{"id": "p1", "name": "Studio sluchátka", "price_cents": 249000, "stock": 12},
			map[string]any{"id": "p2", "name": "Mechanická klávesnice", "price_cents": 329000, "stock": 5},
			map[string]any{"id": "p3", "name": "USB-C rozbočovač", "price_cents": 129000, "stock": 2},
			map[string]any{"id": "p4", "name": "Webkamera", "price_cents": 189000, "stock": 0},
		},
	)

	flow.Seed(
		"Users",
		[]any{
			map[string]any{"id": "u1", "name": "Petra Nováková", "email": "petra@example.test", "country": "CZ"},
			map[string]any{"id": "u2", "name": "David Miller", "email": "david@example.test", "country": "US"},
			map[string]any{"id": "u3", "name": "Nora Silva", "email": "nora@example.test", "country": "BR"},
		},
	)

	flow.Type("DeviceID", flow.String(), flow.Between(flow.Length(flow.Ref("value")), 1, 64))

	flow.Type("Battery", flow.Integer(), flow.Between(flow.Ref("value"), 0, 100))

	flow.Type("Latitude", flow.Number(), flow.Between(flow.Ref("value"), -90, 90))

	flow.Type("Longitude", flow.Number(), flow.Between(flow.Ref("value"), -180, 180))

	flow.Mqtt(
		"DeviceStatus",
		flow.Topic("devices/{device_id}/status"),
		flow.Input("device_id", flow.Named("DeviceID")),
		flow.Payload(map[string]any{"online": flow.Boolean(), "battery": flow.Named("Battery")}),
		flow.History(24, "hours"),
	)

	flow.Mqtt(
		"DevicePosition",
		flow.Topic("devices/{device_id}/position"),
		flow.Input("device_id", flow.Named("DeviceID")),
		flow.Payload(map[string]any{"latitude": flow.Named("Latitude"), "longitude": flow.Named("Longitude")}),
		flow.History(5, "minutes"),
	)

	flow.Table(
		"DeviceAlerts",
		map[string]any{
			"id":         flow.String(),
			"device_id":  flow.Named("DeviceID"),
			"kind":       flow.String(),
			"created_at": flow.String(),
		},
	)

	flow.OnMqtt(
		"DeviceStatus",
		flow.When(
			flow.Less(flow.Ref("message.battery"), 20),
			flow.Transaction(
				flow.Insert(
					"DeviceAlerts",
					map[string]any{
						"id":         flow.Uuid("alert"),
						"device_id":  flow.Ref("input.device_id"),
						"kind":       "low_battery",
						"created_at": flow.Ref("request.time"),
					},
				),
				flow.Commit(),
			),
		),
		flow.Result(map[string]any{"accepted": true}),
	)

	flow.Http(
		"GET",
		"/api/device-alerts/:device_id",
		flow.Input("device_id", flow.Named("DeviceID")),
		flow.Bind(
			"alerts",
			flow.Query("DeviceAlerts", "a", flow.Equal(flow.Ref("a.device_id"), flow.Ref("input.device_id"))),
		),
		flow.Result(map[string]any{"alerts": flow.Ref("alerts")}),
	)

	flow.Http(
		"GET",
		"/api/devices/:device_id",
		flow.Input("device_id", flow.Named("DeviceID")),
		flow.Bind("status", flow.Last(flow.Source("DeviceStatus", flow.Ref("input.device_id")))),
		flow.Bind(
			"positions",
			flow.OrderBy(
				flow.QuerySource(
					"DevicePosition",
					flow.Ref("input.device_id"),
					"p",
					flow.GreaterEqual(flow.Ref("p.received_at"), flow.Ago(5, "minutes")),
				),
				"p.received_at",
				"asc",
			),
		),
		flow.Result(
			map[string]any{
				"device_id": flow.Ref("input.device_id"),
				"status":    flow.Ref("status"),
				"positions": flow.Ref("positions"),
			},
		),
	)

	flow.Http(
		"POST",
		"/api/orders",
		flow.Input("user_id", flow.Named("UserID")),
		flow.Input("items", flow.Named("Cart")),
		flow.Input("payment_method", flow.Named("PaymentMethod")),
		flow.Input("email_failure", flow.Named("Bool"), false),
		flow.Transaction(
			flow.Bind(
				"user",
				flow.One(flow.Query("Users", "u", flow.Equal(flow.Ref("u.id"), flow.Ref("input.user_id")))),
			),
			flow.Ensure(
				flow.Present(flow.Ref("user")),
				flow.Error(404, "user_not_found", "Uživatel neexistuje."),
			),
			flow.Bind("cart", flow.GroupSum(flow.Ref("input.items"), "product_id", "quantity")),
			flow.Bind(
				"snapshots",
				flow.Typed(
					flow.ListOf(flow.Named("OrderItem")),
					flow.MapEach(
						"item",
						flow.Ref("cart"),
						flow.Bind(
							"product",
							flow.One(
								flow.Query(
									"Products",
									"p",
									flow.Equal(flow.Ref("p.id"), flow.Ref("item.product_id")),
								),
							),
						),
						flow.Ensure(
							flow.Present(flow.Ref("product")),
							flow.Error(404, "product_not_found", "Produkt neexistuje."),
						),
						flow.Result(
							map[string]any{
								"id":          flow.Ref("product.id"),
								"name":        flow.Ref("product.name"),
								"price_cents": flow.Ref("product.price_cents"),
								"quantity":    flow.Ref("item.quantity"),
							},
						),
					),
				),
			),
			flow.Bind(
				"total",
				flow.Typed(
					flow.Integer(),
					flow.Sum(
						"item",
						flow.Ref("snapshots"),
						flow.Multiply(flow.Ref("item.price_cents"), flow.Ref("item.quantity")),
					),
				),
			),
			flow.Bind(
				"order",
				flow.Typed(
					flow.Named("Order"),
					flow.Insert(
						"Orders",
						map[string]any{
							"id":             flow.Uuid("ord"),
							"user_id":        flow.Ref("user.id"),
							"request_id":     flow.Ref("request.id"),
							"created_at":     flow.Ref("request.time"),
							"items":          flow.Ref("snapshots"),
							"total_cents":    flow.Ref("total"),
							"payment_method": flow.Ref("input.payment_method"),
							"payment_url":    "",
							"status":         "awaiting_payment",
						},
					),
				),
			),
			flow.Each(
				"item",
				flow.Ref("snapshots"),
				flow.Bind(
					"product",
					flow.One(flow.Query("Products", "p", flow.Equal(flow.Ref("p.id"), flow.Ref("item.id")))),
				),
				flow.Ensure(
					flow.Present(flow.Ref("product")),
					flow.Error(404, "product_not_found", "Produkt neexistuje."),
				),
				flow.Ensure(
					flow.GreaterEqual(flow.Ref("product.stock"), flow.Ref("item.quantity")),
					flow.Error(409, "insufficient_stock", "Nedostatek kusů na skladě."),
				),
				flow.Update(
					flow.Query("Products", "p", flow.Equal(flow.Ref("p.id"), flow.Ref("item.id"))),
					map[string]any{"stock": flow.Subtract(flow.Ref("p.stock"), flow.Ref("item.quantity"))},
				),
			),
			flow.Bind(
				"payment",
				flow.Call(
					"Payment.create_url",
					map[string]any{
						"order_id":     flow.Ref("order.id"),
						"amount_cents": flow.Ref("order.total_cents"),
						"currency":     "CZK",
						"country":      flow.Ref("user.country"),
						"method":       flow.Ref("input.payment_method"),
					},
				),
			),
			flow.Update(
				flow.Query("Orders", "o", flow.Equal(flow.Ref("o.id"), flow.Ref("order.id"))),
				map[string]any{"payment_url": flow.Ref("payment.url")},
			),
			flow.Queue(
				"Email.send_confirmation",
				map[string]any{
					"order_id":         flow.Ref("order.id"),
					"to":               flow.Ref("user.email"),
					"subject":          flow.Concat([]any{"Objednávka ", flow.Ref("order.id")}),
					"payment_url":      flow.Ref("payment.url"),
					"total_cents":      flow.Ref("order.total_cents"),
					"items":            flow.Ref("snapshots"),
					"simulate_failure": flow.Ref("input.email_failure"),
				},
				flow.Policy(flow.Attempts(3), flow.Delay(1000, "ms"), flow.Retain()),
			),
			flow.Commit(),
		),
		flow.Response(
			201,
			map[string]any{
				"order": flow.Merge(
					flow.Ref("order"),
					map[string]any{"currency": "CZK", "payment_url": flow.Ref("payment.url")},
				),
				"payment": flow.Ref("payment"),
				"email":   map[string]any{"state": "queued"},
			},
		),
	)

	flow.Http(
		"GET",
		"/api/orders/:order_id",
		flow.Input("order_id", flow.Named("String")),
		flow.Bind(
			"order",
			flow.One(flow.Query("Orders", "o", flow.Equal(flow.Ref("o.id"), flow.Ref("input.order_id")))),
		),
		flow.Ensure(
			flow.Present(flow.Ref("order")),
			flow.Error(404, "order_not_found", "Objednávka neexistuje."),
		),
		flow.Result(flow.Ref("order")),
	)

	flow.Type(
		"ProductPhoto",
		flow.Image(),
		flow.All(
			flow.LessEqual(flow.Ref("value.byte_size"), 10485760),
			flow.LessEqual(flow.Ref("value.width"), 8000),
			flow.LessEqual(flow.Ref("value.height"), 8000),
		),
	)

	flow.Type("PhotoSize", flow.Integer(), flow.Between(flow.Ref("value"), 1, 4096))

	flow.Table(
		"Photos",
		map[string]any{
			"id":         flow.String(),
			"product_id": flow.String(),
			"file_id":    flow.String(),
			"url":        flow.String(),
			"width":      flow.Integer(),
			"height":     flow.Integer(),
		},
	)

	flow.Http(
		"POST",
		"/api/products/:product_id/photo",
		flow.Input("product_id", flow.Named("String")),
		flow.Input("photo", flow.Named("ProductPhoto")),
		flow.Input("width", flow.Named("PhotoSize"), 800),
		flow.Input("height", flow.Named("PhotoSize"), 600),
		flow.Transaction(
			flow.Bind(
				"product",
				flow.One(
					flow.Query("Products", "p", flow.Equal(flow.Ref("p.id"), flow.Ref("input.product_id"))),
				),
			),
			flow.Ensure(
				flow.Present(flow.Ref("product")),
				flow.Error(404, "product_not_found", "Produkt neexistuje."),
			),
			flow.Bind(
				"resized",
				flow.Typed(
					flow.Named("ProductPhoto"),
					flow.Call(
						"Image.resize",
						map[string]any{
							"image":  flow.Ref("input.photo"),
							"width":  flow.Ref("input.width"),
							"height": flow.Ref("input.height"),
						},
					),
				),
			),
			flow.Bind("stored", flow.Call("Files.put", map[string]any{"file": flow.Ref("resized")})),
			flow.Bind(
				"saved",
				flow.Insert(
					"Photos",
					map[string]any{
						"id":         flow.Uuid("photo"),
						"product_id": flow.Ref("input.product_id"),
						"file_id":    flow.Ref("stored.id"),
						"url":        flow.Ref("stored.url"),
						"width":      flow.Ref("resized.width"),
						"height":     flow.Ref("resized.height"),
					},
				),
			),
			flow.Commit(),
		),
		flow.Response(201, map[string]any{"photo": flow.Ref("saved"), "file": flow.Ref("stored")}),
	)

	flow.Http(
		"GET",
		"/api/products/:product_id/photos",
		flow.Input("product_id", flow.Named("String")),
		flow.Bind(
			"photos",
			flow.Query("Photos", "p", flow.Equal(flow.Ref("p.product_id"), flow.Ref("input.product_id"))),
		),
		flow.Result(map[string]any{"photos": flow.Ref("photos")}),
	)

	flow.Table(
		"DeviceAccess",
		map[string]any{
			"id":        flow.String(),
			"user_id":   flow.Named("UserID"),
			"token":     flow.String(),
			"device_id": flow.Named("DeviceID"),
		},
	)

	flow.Seed(
		"DeviceAccess",
		[]any{
			map[string]any{"id": "petra-mower1", "user_id": "u1", "token": "demo-petra", "device_id": "mower1"},
			map[string]any{"id": "david-mower2", "user_id": "u2", "token": "demo-david", "device_id": "mower2"},
		},
	)

	flow.Websocket(
		"/ws",
		flow.Input("token", flow.Named("String")),
		flow.Authorize(
			flow.Exists(
				flow.Query(
					"DeviceAccess",
					"access",
					flow.All(
						flow.Equal(flow.Ref("access.token"), flow.Ref("input.token")),
						flow.Exists(
							flow.Query(
								"Users",
								"user",
								flow.Equal(flow.Ref("user.id"), flow.Ref("access.user_id")),
							),
						),
					),
				),
			),
		),
		flow.AllowSource(
			"DeviceStatus",
			flow.Exists(
				flow.Query(
					"DeviceAccess",
					"access",
					flow.All(
						flow.Equal(flow.Ref("access.token"), flow.Ref("input.token")),
						flow.Equal(flow.Ref("access.device_id"), flow.Ref("input.device_id")),
					),
				),
			),
		),
		flow.AllowSource(
			"DevicePosition",
			flow.Exists(
				flow.Query(
					"DeviceAccess",
					"access",
					flow.All(
						flow.Equal(flow.Ref("access.token"), flow.Ref("input.token")),
						flow.Equal(flow.Ref("access.device_id"), flow.Ref("input.device_id")),
					),
				),
			),
		),
	)

	flow.Type("MowerAction", flow.String(), flow.Member(flow.Ref("value"), []any{"start", "stop"}))

	flow.Mqtt(
		"DeviceCommand",
		flow.Topic("devices/{device_id}/command"),
		flow.Input("device_id", flow.Named("DeviceID")),
		flow.Payload(map[string]any{"command_id": flow.String(), "action": flow.Named("MowerAction")}),
		flow.History(24, "hours"),
	)

	flow.Http(
		"POST",
		"/api/devices/:device_id/commands",
		flow.Input("device_id", flow.Named("DeviceID")),
		flow.Input("action", flow.Named("MowerAction")),
		flow.Input("token", flow.Named("String")),
		flow.Ensure(
			flow.Exists(
				flow.Query(
					"DeviceAccess",
					"access",
					flow.All(
						flow.Equal(flow.Ref("access.token"), flow.Ref("input.token")),
						flow.Equal(flow.Ref("access.device_id"), flow.Ref("input.device_id")),
						flow.Exists(
							flow.Query(
								"Users",
								"user",
								flow.Equal(flow.Ref("user.id"), flow.Ref("access.user_id")),
							),
						),
					),
				),
			),
			flow.Error(403, "forbidden", "Přístup k zařízení je zamítnut."),
		),
		flow.Transaction(
			flow.Bind(
				"outgoing",
				flow.Publish(
					flow.Source("DeviceCommand", flow.Ref("input.device_id")),
					map[string]any{"command_id": flow.Uuid("command"), "action": flow.Ref("input.action")},
				),
			),
			flow.Commit(),
		),
		flow.Response(202, flow.Ref("outgoing")),
	)
}
