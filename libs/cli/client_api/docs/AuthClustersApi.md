# \AuthClustersApi

All URIs are relative to *http://localhost*

Method | HTTP request | Description
------------- | ------------- | -------------
[**callback_login**](AuthClustersApi.md#callback_login) | **GET** /clusters/{ns}/{cluster}/auth/callback | Callback from the cluster's OIDC provider
[**cluster_login**](AuthClustersApi.md#cluster_login) | **GET** /clusters/{ns}/{cluster}/auth/login | Redirect to the cluster's login page



## callback_login

> models::CallbackModel callback_login(ns, cluster, x_front_callback, x_kubectl_callback, code, state)
Callback from the cluster's OIDC provider

If the cluster is not found or disabled, return 404

### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**ns** | **String** | Namespace | [required] |
**cluster** | **String** | Cluster name | [required] |
**x_front_callback** | Option<**String**> | If it's from the frontend, this header will be set. | [required] |
**x_kubectl_callback** | Option<**String**> | If it's from kubectl plugin, this header will be set. | [required] |
**code** | **String** | Authorization code from the OIDC provider | [required] |
**state** | **String** | State parameter to prevent CSRF | [required] |

### Return type

[**models::CallbackModel**](CallbackModel.md)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## cluster_login

> String cluster_login(ns, cluster, x_front_callback, x_kubectl_callback)
Redirect to the cluster's login page

If the cluster is not found or disabled, return 404

### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**ns** | **String** | Namespace | [required] |
**cluster** | **String** | Cluster name | [required] |
**x_front_callback** | Option<**String**> | If it's from the frontend, this header will be set. | [required] |
**x_kubectl_callback** | Option<**String**> | If it's from kubectl plugin, this header will be set. | [required] |

### Return type

**String**

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: text/plain

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

