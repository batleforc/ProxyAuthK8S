# SessionTokens

## Properties

Name | Type | Description | Notes
------------ | ------------- | ------------- | -------------
**access_token** | **String** |  |
**expires_in** | Option<**i64**> | Seconds until `access_token` expires, when the IdP said. | [optional]
**refresh_token** | Option<**String**> | Absent when the IdP issued none (no session to renew). | [optional]
**token_type** | **String** |  |

[[Back to Model list]](../README.md#documentation-for-models) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to README]](../README.md)


