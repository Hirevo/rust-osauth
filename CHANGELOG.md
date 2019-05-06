# Change Log

## 0.2.1 (2019-04-19)

#### Features

* **session:**  add get_query and get_json_query ([3e14f4fa](https://github.com/dtantsur/rust-osauth/commit/3e14f4fac70d48ab0b00350750ea210623975738))

## 0.2.0 (2019-04-11)

#### Breaking Changes

* **services:**
  *  change IMAGE and NETWORK to have their own types ([f6c38f33](https://github.com/dtantsur/rust-osauth/commit/f6c38f33a790537770d81a95c9e5e175ed4a5946))
  *  change set_api_version_headers to accept HeaderMap ([b6edf6b9](https://github.com/dtantsur/rust-osauth/commit/b6edf6b976860fa3e55c679c6341bb483843a00d))

#### Features

* **services:**
  *  support for object and block storage services ([da885d09](https://github.com/dtantsur/rust-osauth/commit/da885d090c386a3973ab4ab1629e1a8cc09060b8))

## 0.1.1 (2019-03-31)

#### Bug Fixes

* **session:**  short-cut pick\_api\_version on empty input ([744a5102](https://github.com/dtantsur/rust-osauth/commit/744a510228674b40b9d512e5f75d0488f19639fe))

#### Features

* **session:**
  *  accept IntoIterator in `pick_api_version` ([d19a4201](https://github.com/dtantsur/rust-osauth/commit/d19a42016ff85bc573d829c25d0d7bdbe3e6fd7a))
  *  add `refresh`, `set_auth_type` and `with_auth_type` ([80ea7579](https://github.com/dtantsur/rust-osauth/commit/80ea7579938e742930f938ea610530978bf99b4b))


## 0.1.0 (2019-03-16)

Initial version.