use flow_runtime::engine::{Config, Runtime};
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;

fn key() -> &'static [u8] {
    b"development-key-32-bytes-minimum-123456"
}

fn token(aud: &str, sub: &str) -> String {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    let head = URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256"}"#);
    let payload = URL_SAFE_NO_PAD.encode(
        json!({
            "iss": "https://identity.example.com",
            "aud": aud,
            "sub": sub,
            "exp": chrono::Utc::now().timestamp() + 300
        })
        .to_string(),
    );
    let data = format!("{head}.{payload}");
    let signature = ring::hmac::sign(
        &ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key()),
        data.as_bytes(),
    );
    format!(
        "Bearer {data}.{}",
        URL_SAFE_NO_PAD.encode(signature.as_ref())
    )
}

fn open(name: &str, aliases: &[&str]) -> Runtime {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("projects")
        .join(name)
        .join("application.flow");
    let source =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let mut config = Config::default();
    for alias in aliases {
        config.jwt_keys.insert((*alias).into(), key().to_vec());
    }
    if name == "demo" {
        config.event_credentials.insert(
            "service:1".into(),
            "ApiKey automation-key-long-enough-123456789".into(),
        );
    }
    Runtime::open(&source, ":memory:", config).unwrap_or_else(|e| panic!("compile {name}: {e}"))
}

fn err_code(result: Result<Value, flow_runtime::Error>) -> String {
    match result {
        Ok(v) => panic!("expected error, got {v}"),
        Err(e) => e.code.to_string(),
    }
}

fn ids(value: &Value) -> Vec<i64> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_i64().unwrap())
        .collect()
}

#[test]
fn clinic_scopes_visits_and_notes() {
    let mut r = open("clinic", &["user"]);
    let petra = token("clinic", "idp:u1");
    let doctor = token("clinic", "idp:doc");
    let desk = token("clinic", "idp:desk");
    assert_eq!(
        err_code(r.execute("MyVisits", json!({"userId": 2}), Some(&petra))),
        "not_found"
    );
    let petra_visits = r
        .execute("MyVisits", json!({"userId": 1}), Some(&petra))
        .unwrap();
    assert_eq!(ids(&petra_visits), vec![1, 4]);
    let doctor_petra = r
        .execute("MyVisits", json!({"userId": 1}), Some(&doctor))
        .unwrap();
    assert_eq!(ids(&doctor_petra), vec![1]);
    let desk_david = r
        .execute("MyVisits", json!({"userId": 2}), Some(&desk))
        .unwrap();
    assert_eq!(ids(&desk_david), vec![5, 2]);
    let booked = r
        .execute(
            "BookVisit",
            json!({"patientId": 1, "slotId": 2}),
            Some(&petra),
        )
        .unwrap();
    assert_eq!(booked["status"], "SCHEDULED");
    assert_eq!(
        err_code(r.execute(
            "BookVisit",
            json!({"patientId": 2, "slotId": 4}),
            Some(&petra),
        )),
        "forbidden"
    );
    let desk_book = r
        .execute(
            "BookVisit",
            json!({"patientId": 2, "slotId": 4}),
            Some(&desk),
        )
        .unwrap();
    assert_eq!(desk_book["status"], "SCHEDULED");
    let note = r
        .execute(
            "AddNote",
            json!({"visitId": 1, "body": "follow-up"}),
            Some(&doctor),
        )
        .unwrap();
    assert_eq!(note["body"], "follow-up");
    assert_eq!(
        err_code(r.execute(
            "AddNote",
            json!({"visitId": 2, "body": "other practitioner"}),
            Some(&doctor),
        )),
        "forbidden"
    );
    assert_eq!(
        err_code(r.execute(
            "AddNote",
            json!({"visitId": 1, "body": "reception"}),
            Some(&desk),
        )),
        "forbidden"
    );
}

#[test]
fn jobs_blocks_impersonation_and_closed_listings() {
    let mut r = open("jobs", &["user"]);
    let nora = token("jobs", "idp:u1");
    let hr = token("jobs", "idp:hr");
    let own = r
        .execute("MyApplications", json!({"userId": 1}), Some(&nora))
        .unwrap();
    assert_eq!(ids(&own), vec![1]);
    assert_eq!(
        err_code(r.execute("MyApplications", json!({"userId": 2}), Some(&nora))),
        "not_found"
    );
    assert_eq!(
        err_code(r.execute(
            "Apply",
            json!({"listingId": 5, "coverLetter": "closed"}),
            Some(&nora),
        )),
        "forbidden"
    );
    let message = r
        .execute(
            "MessageCandidate",
            json!({"applicationId": 1, "body": "from candidate"}),
            Some(&nora),
        )
        .unwrap();
    assert_eq!(message["body"], "from candidate");
    assert_eq!(
        err_code(r.execute(
            "MessageCandidate",
            json!({"applicationId": 2, "body": "cross company"}),
            Some(&hr),
        )),
        "forbidden"
    );
    let listing = r
        .execute("Listing", json!({"listingId": 1}), Some(&nora))
        .unwrap();
    assert!(listing.get("applications").is_none());
}

#[test]
fn school_requires_active_enrollment() {
    let mut r = open("school", &["user"]);
    let eva = token("school", "idp:s1");
    let lea = token("school", "idp:s3");
    assert_eq!(
        r.execute("CourseAssignments", json!({"courseId": 3}), Some(&lea))
            .unwrap(),
        json!([])
    );
    assert_eq!(
        err_code(r.execute(
            "SubmitWork",
            json!({"assignmentId": 4, "body": "not enrolled"}),
            Some(&eva),
        )),
        "forbidden"
    );
    let submitted = r
        .execute(
            "SubmitWork",
            json!({"assignmentId": 1, "body": "hello"}),
            Some(&eva),
        )
        .unwrap();
    assert_eq!(submitted["status"], "SUBMITTED");
}

#[test]
fn tracker_hides_foreign_tickets() {
    let mut r = open("tracker", &["user"]);
    let petra = token("tracker", "idp:u1");
    let tickets = r
        .execute("ProjectTickets", json!({"projectId": 1}), Some(&petra))
        .unwrap();
    assert_eq!(ids(&tickets), vec![1]);
    assert_eq!(
        err_code(r.execute("Ticket", json!({"ticketId": 2}), Some(&petra))),
        "not_found"
    );
}

#[test]
fn rideshare_blocks_trip_hijack_and_foreign_fare() {
    let mut r = open("rideshare", &["user", "driver"]);
    let petra = token("rideshare", "idp:u1");
    let driver = token("rideshare", "idp:d2");
    let trips = r
        .execute("MyTrips", json!({"userId": 1}), Some(&petra))
        .unwrap();
    assert_eq!(ids(&trips), vec![5, 1]);
    assert_eq!(
        err_code(r.execute("Trip", json!({"tripId": 2}), Some(&petra))),
        "not_found"
    );
    let requested = r
        .execute(
            "RequestTrip",
            json!({
                "pickupLat": 50.08,
                "pickupLng": 14.42,
                "dropoffLat": 50.09,
                "dropoffLng": 14.43
            }),
            Some(&petra),
        )
        .unwrap();
    assert_eq!(requested["fare"], 89.0);
    assert_eq!(requested["status"], "REQUESTED");
    let accepted = r
        .execute(
            "AcceptTrip",
            json!({"tripId": requested["id"].as_i64().unwrap()}),
            Some(&driver),
        )
        .unwrap();
    assert_eq!(accepted["status"], "MATCHED");
    assert_eq!(
        err_code(r.execute("AcceptTrip", json!({"tripId": 1}), Some(&driver))),
        "forbidden"
    );
    assert_eq!(
        err_code(r.execute("AcceptTrip", json!({"tripId": 4}), Some(&driver))),
        "forbidden"
    );
}

#[test]
fn hotel_gym_library_and_restaurant_grants() {
    let mut hotel = open("hotel", &["user"]);
    let david = token("hotel", "idp:u1");
    let desk = token("hotel", "idp:desk");
    let petra = token("hotel", "idp:u2");
    let stays = hotel
        .execute("MyStays", json!({"userId": 1}), Some(&david))
        .unwrap();
    assert_eq!(ids(&stays), vec![4, 1]);
    assert_eq!(
        err_code(hotel.execute("CheckIn", json!({"stayId": 5}), Some(&desk))),
        "forbidden"
    );
    let checked = hotel
        .execute("CheckIn", json!({"stayId": 3}), Some(&desk))
        .unwrap();
    assert_eq!(checked["status"], "CHECKED_IN");
    let first = hotel
        .execute(
            "BookStay",
            json!({
                "roomId": 1,
                "nights": 1,
                "guests": 1,
                "extras": [],
                "checkIn": "2026-12-10T15:00:00Z"
            }),
            Some(&david),
        )
        .unwrap();
    assert_eq!(first["status"], "REQUESTED");
    assert_eq!(
        err_code(hotel.execute(
            "BookStay",
            json!({
                "roomId": 1,
                "nights": 1,
                "guests": 1,
                "extras": [],
                "checkIn": "2026-12-10T15:00:00Z"
            }),
            Some(&petra),
        )),
        "forbidden"
    );

    let mut gym = open("gym", &["user"]);
    let member = token("gym", "idp:u1");
    let frozen = token("gym", "idp:u3");
    let staff = token("gym", "idp:st");
    assert_eq!(
        err_code(gym.execute("BookClass", json!({"sessionId": 1}), Some(&frozen))),
        "forbidden"
    );
    assert_eq!(
        err_code(gym.execute("BookClass", json!({"sessionId": 1}), Some(&member))),
        "forbidden"
    );
    let booked = gym
        .execute("BookClass", json!({"sessionId": 3}), Some(&member))
        .unwrap();
    assert!(booked["id"].as_i64().unwrap() > 0);
    let check_in = gym
        .execute("CheckInMember", json!({"memberId": 1}), Some(&staff))
        .unwrap();
    assert!(check_in.get("member").is_none());

    let mut library = open("library", &["user"]);
    let patron = token("library", "idp:u1");
    let lib = token("library", "idp:lib");
    let loans = library
        .execute("MyLoans", json!({"userId": 1}), Some(&patron))
        .unwrap();
    assert_eq!(ids(&loans), vec![1, 5]);
    let returned = library
        .execute("ReturnCopy", json!({"loanId": 1}), Some(&lib))
        .unwrap();
    assert_eq!(returned["status"], "RETURNED");
    let hold = library
        .execute("PlaceHold", json!({"workId": 2}), Some(&patron))
        .unwrap();
    assert_eq!(hold["status"], "WAITING");

    let mut restaurant = open("restaurant", &["user"]);
    let guest = token("restaurant", "idp:u1");
    let host = token("restaurant", "idp:host");
    let reservations = restaurant
        .execute("MyReservations", json!({"userId": 1}), Some(&guest))
        .unwrap();
    assert_eq!(ids(&reservations), vec![5, 1]);
    assert_eq!(
        err_code(restaurant.execute(
            "ConfirmReservation",
            json!({"reservationId": 1}),
            Some(&host),
        )),
        "forbidden"
    );
    let confirmed = restaurant
        .execute(
            "ConfirmReservation",
            json!({"reservationId": 2}),
            Some(&host),
        )
        .unwrap();
    assert_eq!(confirmed["status"], "CONFIRMED");
}

#[test]
fn bookstore_delivery_and_social_use_actor() {
    let mut bookstore = open("bookstore", &["user"]);
    let petra = token("bookstore", "idp:u1");
    let book = bookstore
        .execute("Book", json!({"bookId": 1}), None)
        .unwrap();
    assert_eq!(book["title"], "Babička");
    assert!(book.get("reviews").is_none());
    let order = bookstore
        .execute(
            "PlaceOrder",
            json!({"items": [{"bookId": 2, "quantity": 1}], "paymentMethod": "CARD"}),
            Some(&petra),
        )
        .unwrap();
    assert_eq!(order["status"], "AWAITING_PAYMENT");
    let review = bookstore
        .execute(
            "WriteReview",
            json!({"bookId": 2, "rating": 4.0, "body": "ok"}),
            Some(&petra),
        )
        .unwrap();
    assert_eq!(review["body"], "ok");

    let mut delivery = open("delivery", &["user", "courier"]);
    let customer = token("delivery", "idp:u1");
    let courier = token("delivery", "idp:c2");
    let placed = delivery
        .execute(
            "PlaceOrder",
            json!({"kitchenId": 1, "items": [{"itemId": 1, "quantity": 1}]}),
            Some(&customer),
        )
        .unwrap();
    assert_eq!(placed["status"], "PLACED");
    let assigned = delivery
        .execute(
            "AssignCourier",
            json!({"orderId": placed["id"].as_i64().unwrap(), "courierId": 2}),
            Some(&courier),
        )
        .unwrap();
    assert_eq!(assigned["status"], "OUT_FOR_DELIVERY");
    assert_eq!(
        err_code(delivery.execute(
            "AssignCourier",
            json!({"orderId": 1, "courierId": 2}),
            Some(&courier),
        )),
        "forbidden"
    );

    let mut social = open("social-network", &["user"]);
    let author = token("social", "idp:u1");
    let like = social
        .execute("LikePost", json!({"postId": 2}), Some(&author))
        .unwrap();
    assert!(like["id"].as_i64().unwrap() > 0);
    let follow = social
        .execute("FollowUser", json!({"followeeId": 3}), Some(&author))
        .unwrap();
    assert!(follow["id"].as_i64().unwrap() > 0);
}
