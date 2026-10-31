# ReadinessBody

## Properties

Name | Type | Description | Notes
------------ | ------------- | ------------- | -------------
**controller** | **String** | `failing` while the controller's `ProxyKubeApi` watch has kept failing for 5 minutes, `ok` otherwise. Informational: it never makes the pod unready, the proxy keeps serving from Redis. |
**oidc** | **String** | `ok` once the service-wide OIDC provider has been discovered, `pending` while the boot-time discovery is still retrying. |
**redis** | **String** | `ok` when Redis answered a `PING`, `unavailable` otherwise. |

[[Back to Model list]](../README.md#documentation-for-models) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to README]](../README.md)


