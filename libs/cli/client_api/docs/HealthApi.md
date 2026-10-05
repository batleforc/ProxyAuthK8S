# \HealthApi

All URIs are relative to *http://localhost*

Method | HTTP request | Description
------------- | ------------- | -------------
[**health**](HealthApi.md#health) | **GET** /management/health | Liveness: the process is up and serving HTTP.
[**ready**](HealthApi.md#ready) | **GET** /management/ready | Readiness: the pod can serve proxied traffic.



## health

> health()
Liveness: the process is up and serving HTTP.

Deliberately checks no dependency, so a Redis or IdP outage never makes Kubernetes restart otherwise-healthy pods. Use `/management/ready` to decide whether a pod should receive traffic.

### Parameters

This endpoint does not need any parameter.

### Return type

 (empty response body)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: Not defined

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## ready

> models::ReadinessBody ready()
Readiness: the pod can serve proxied traffic.

Every proxied request needs Redis (cluster registry, sessions, throttling), so the pod is only ready when Redis answers a `PING` within 2 seconds.

### Parameters

This endpoint does not need any parameter.

### Return type

[**models::ReadinessBody**](ReadinessBody.md)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

