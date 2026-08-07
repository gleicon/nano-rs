//! Unit tests for VirtualHostRouter — extracted from src/http/router.rs
//! Tests using private functions (handle_ws_upgrade) remain embedded.
use nano::http::{AppState, HandlerType, RouteTarget, VirtualHostRouter};
use std::sync::Arc;

#[test]
fn test_router_exact_match() {
    let default = RouteTarget {
        hostname: "default".to_string(),
        handler_type: HandlerType::StaticResponse("default".to_string()),
    };
    let mut router = VirtualHostRouter::new(default);

    let api_target = RouteTarget {
        hostname: "api.example.com".to_string(),
        handler_type: HandlerType::StaticResponse("api".to_string()),
    };
    router.register("api.example.com".to_string(), api_target);

    let resolved = router.resolve("api.example.com");
    assert!(matches!(resolved.handler_type, HandlerType::StaticResponse(ref s) if s == "api"));

    let resolved_upper = router.resolve("API.EXAMPLE.COM");
    assert!(
        matches!(resolved_upper.handler_type, HandlerType::StaticResponse(ref s) if s == "api")
    );
}

#[test]
fn test_router_fallback() {
    let default = RouteTarget {
        hostname: "default".to_string(),
        handler_type: HandlerType::StaticResponse("fallback".to_string()),
    };
    let router = VirtualHostRouter::new(default);

    let resolved = router.resolve("unknown.host.com");
    assert!(matches!(resolved.handler_type, HandlerType::StaticResponse(ref s) if s == "fallback"));
}

#[test]
fn test_router_default_constructor() {
    let router = VirtualHostRouter::default();
    let resolved = router.resolve("any.host.com");
    assert!(
        matches!(resolved.handler_type, HandlerType::StaticResponse(ref s) if s == "NANO Runtime")
    );
}

#[test]
fn test_case_insensitive_variations() {
    let default = RouteTarget {
        hostname: "default".to_string(),
        handler_type: HandlerType::StaticResponse("default".to_string()),
    };
    let mut router = VirtualHostRouter::new(default);

    router.register(
        "Test.Host.COM".to_string(),
        RouteTarget {
            hostname: "Test.Host.COM".to_string(),
            handler_type: HandlerType::StaticResponse("test".to_string()),
        },
    );

    for case in &[
        "test.host.com",
        "TEST.HOST.COM",
        "Test.Host.COM",
        "tEsT.hOsT.cOm",
    ] {
        let resolved = router.resolve(case);
        assert!(
            matches!(resolved.handler_type, HandlerType::StaticResponse(ref s) if s == "test"),
            "Failed to match case: {}",
            case
        );
    }
}

#[test]
fn test_multiple_routes() {
    let default = RouteTarget {
        hostname: "default".to_string(),
        handler_type: HandlerType::StaticResponse("default".to_string()),
    };
    let mut router = VirtualHostRouter::new(default);

    router.register(
        "api.example.com".to_string(),
        RouteTarget {
            hostname: "api.example.com".to_string(),
            handler_type: HandlerType::StaticResponse("api".to_string()),
        },
    );
    router.register(
        "blog.example.com".to_string(),
        RouteTarget {
            hostname: "blog.example.com".to_string(),
            handler_type: HandlerType::StaticResponse("blog".to_string()),
        },
    );

    assert!(
        matches!(router.resolve("api.example.com").handler_type, HandlerType::StaticResponse(ref s) if s == "api")
    );
    assert!(
        matches!(router.resolve("blog.example.com").handler_type, HandlerType::StaticResponse(ref s) if s == "blog")
    );
    assert!(
        matches!(router.resolve("other.com").handler_type, HandlerType::StaticResponse(ref s) if s == "default")
    );
}

#[test]
fn test_javascript_entrypoint_handler() {
    let default = RouteTarget {
        hostname: "default".to_string(),
        handler_type: HandlerType::StaticResponse("default".to_string()),
    };
    let mut router = VirtualHostRouter::new(default);

    router.register(
        "js.example.com".to_string(),
        RouteTarget {
            hostname: "js.example.com".to_string(),
            handler_type: HandlerType::WinterTCHandler("/app/index.js".to_string()),
        },
    );

    let resolved = router.resolve("js.example.com");
    assert!(
        matches!(resolved.handler_type, HandlerType::WinterTCHandler(ref s) if s == "/app/index.js")
    );
}

#[test]
fn test_sliver_handler_routing() {
    let default = RouteTarget {
        hostname: "default".to_string(),
        handler_type: HandlerType::StaticResponse("default".to_string()),
    };
    let mut router = VirtualHostRouter::new(default);

    router.register(
        "sliver.example.com".to_string(),
        RouteTarget {
            hostname: "sliver.example.com".to_string(),
            handler_type: HandlerType::WinterTCSliverHandler {
                entrypoint: "/app/index.js".to_string(),
                hostname: "sliver.example.com".to_string(),
            },
        },
    );

    let resolved = router.resolve("sliver.example.com");
    match &resolved.handler_type {
        HandlerType::WinterTCSliverHandler {
            entrypoint,
            hostname,
        } => {
            assert_eq!(entrypoint, "/app/index.js");
            assert_eq!(hostname, "sliver.example.com");
        }
        _ => panic!("Expected WinterTCSliverHandler"),
    }
}

#[test]
fn test_appstate_creation() {
    let router = VirtualHostRouter::default();
    let state = Arc::new(AppState::new(router, 2));
    drop(state); // just verify it creates without panic
}
